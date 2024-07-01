use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bitcoin::hashes::Hash;
use bitcoin::{Address, Network};
use candid::{CandidType, Deserialize};
use did::{H160, H256};
use eth_signer::sign_strategy::TransactionSigner;
use ic_exports::ic_cdk::api::management_canister::bitcoin::{GetUtxosResponse, Utxo};
use ic_exports::ic_kit::ic;
use ic_stable_structures::CellStructure;
use ic_task_scheduler::scheduler::TaskScheduler;
use ic_task_scheduler::task::TaskOptions;
use minter_contract_utils::operation_store::MinterOperationId;
use minter_did::id256::Id256;
use minter_did::order::{MintOrder, SignedMintOrder};

use crate::canister::{get_operations_store, get_scheduler, get_state};
use crate::core::index_provider::{OrdIndexProvider, RuneIndexProvider};
use crate::core::utxo_provider::{IcUtxoProvider, UtxoProvider};
use crate::interface::DepositError;
use crate::key::BtcSignerType;
use crate::operation::{OperationState, RuneOperationStore};
use crate::rune_info::{RuneInfo, RuneName};
use crate::scheduler::{PersistentScheduler, RuneBridgeTask};
use crate::state::State;

static NONCE: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, CandidType, Deserialize)]
pub enum DepositRequestStatus {
    /// Deposit request received but is not yet executed.
    Scheduled,
    /// No utxos containing runes found at the deposit address. Waiting for the utxos to be mined
    /// into a block.
    WaitingForInputs {
        requested_at: u64,
        current_ts: u64,
        next_retry_at: u64,
        waiting_until: u64,
        block_height: u32,
    },
    /// No utxos containing runes found at the deposit address. Deposit operation is cancelled.
    NothingToDeposit {
        block_height: u32,
    },
    /// Utxos with runes are found at the deposit address, but are not confirmed yet. Deposit will
    /// proceed after enough confirmations are received.
    WaitingForConfirmations {
        utxos: Vec<Utxo>,
        current_min_confirmations: u32,
        required_confirmations: u32,
        block_height: u32,
    },
    InvalidAmounts {
        requested_amounts: HashMap<RuneName, u128>,
        actual_amounts: HashMap<RuneName, u128>,
    },
    /// Mint orders are signed by the canister but are not sent to the BftBridge. The user may attempt
    /// to send them by themselves or wait for the canister to retry the operation.
    MintOrdersCreated {
        orders: Vec<MintOrderDetails>,
    },
    Minted {
        amounts: Vec<(RuneName, u128, H256)>,
    },
    InternalError {
        details: String,
    },
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct MintOrderDetails {
    rune_name: RuneName,
    amount: u128,
    status: MintOrderStatus,
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub enum MintOrderStatus {
    Created {
        mint_order: SignedMintOrder,
        nonce: u32,
    },
    Sent {
        mint_order: SignedMintOrder,
        nonce: u32,
        tx_id: H256,
    },
    Completed {
        tx_id: H256,
    },
}

#[derive(Debug, Clone, CandidType, Deserialize)]
pub struct RuneDepositPayload {
    pub dst_address: H160,
    pub erc20_address: H160,
    pub requested_amounts: Option<HashMap<RuneName, u128>>,
    pub request_ts: u64,
    pub status: DepositRequestStatus,
}

impl RuneDepositPayload {
    fn with_status(self, new_status: DepositRequestStatus) -> Self {
        Self {
            status: new_status,
            ..self
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(
            self.status,
            DepositRequestStatus::NothingToDeposit { .. }
                | DepositRequestStatus::InvalidAmounts { .. }
                | DepositRequestStatus::Minted { .. }
                | DepositRequestStatus::InternalError { .. }
        )
    }
}

pub(crate) struct RuneDeposit<
    UTXO: UtxoProvider = IcUtxoProvider,
    INDEX: RuneIndexProvider = OrdIndexProvider,
> {
    state: Rc<RefCell<State>>,
    scheduler: Rc<RefCell<PersistentScheduler>>,
    network: Network,
    signer: BtcSignerType,
    utxo_provider: UTXO,
    index_provider: INDEX,
    operation_store: RuneOperationStore,
}

impl RuneDeposit<IcUtxoProvider, OrdIndexProvider> {
    pub fn new(state: Rc<RefCell<State>>, scheduler: Rc<RefCell<PersistentScheduler>>) -> Self {
        let state_ref = state.borrow();

        let network = state_ref.network();
        let ic_network = state_ref.ic_btc_network();
        let indexer_url = state_ref.indexer_url();
        let signer = state_ref.btc_signer();

        drop(state_ref);

        Self {
            state,
            scheduler,
            network,
            signer,
            utxo_provider: IcUtxoProvider::new(ic_network),
            index_provider: OrdIndexProvider::new(indexer_url),
            operation_store: get_operations_store(),
        }
    }

    pub fn get() -> Self {
        Self::new(get_state(), get_scheduler())
    }
}

impl<UTXO: UtxoProvider, INDEX: RuneIndexProvider> RuneDeposit<UTXO, INDEX> {
    pub fn create_deposit_request(
        &mut self,
        dst_address: H160,
        erc20_address: H160,
        amounts: Option<HashMap<RuneName, u128>>,
    ) -> MinterOperationId {
        let id = self.operation_store.new_operation(
            dst_address.clone(),
            OperationState::Deposit(RuneDepositPayload {
                dst_address: dst_address.clone(),
                erc20_address,
                requested_amounts: amounts,
                request_ts: ic::time(),
                status: DepositRequestStatus::Scheduled,
            }),
        );

        log::trace!(
            "New deposit operation requested for address {}. Operation id: {id}.",
            hex::encode(dst_address.0)
        );

        id
    }

    pub async fn process_deposit_request(&mut self, request_id: MinterOperationId) {
        loop {
            let Some(request) = self.operation_store.get(request_id) else {
                log::error!("Deposit request {request_id} was not found in the request store.");
                return;
            };

            let OperationState::Deposit(payload) = request else {
                log::error!("Request {request_id} was found but is not a deposit request.");
                return;
            };

            log::trace!(
                "Executing deposit operation {request_id} with status {:?}.",
                payload.status
            );

            if let ControlFlow::Break(_) = self.execute_request_step(request_id, payload).await {
                return;
            }
        }
    }

    pub fn complete_mint_request(&mut self, dst_address: H160, order_nonce: u32) {
        let requests = self.operation_store.get_for_address(&dst_address);
        for (request_id, request) in requests {
            if let OperationState::Deposit(payload) = request {
                if let DepositRequestStatus::MintOrdersCreated { mut orders } =
                    payload.status.clone()
                {
                    let mut is_updated = false;
                    for order in &mut orders {
                        if let MintOrderStatus::Sent { nonce, tx_id, .. } = &mut order.status {
                            if *nonce == order_nonce {
                                order.status = MintOrderStatus::Completed {
                                    tx_id: tx_id.clone(),
                                };
                                is_updated = true;
                            }
                        }
                    }

                    if is_updated {
                        if orders.iter().all(|order_info| {
                            matches!(order_info.status, MintOrderStatus::Completed { .. })
                        }) {
                            self.complete_deposit_request(request_id, payload, orders)
                        } else {
                            self.operation_store
                                .update(request_id, OperationState::Deposit(payload));
                        }

                        break;
                    }
                }
            }
        }
    }

    fn complete_deposit_request(
        &mut self,
        request_id: MinterOperationId,
        request: RuneDepositPayload,
        orders: Vec<MintOrderDetails>,
    ) {
        self.update_request_status(request_id, request, DepositRequestStatus::Minted { amounts: orders.into_iter().map(|order_details| {
            let name = order_details.rune_name;
            let amount = order_details.amount;
            let tx_id = match order_details.status {
                MintOrderStatus::Completed { tx_id } => tx_id,
                s => {
                    log::error!("Invalid state of the mint order when completing deposit request: {s:?}");
                    H256::default()
                },
            };

            (name, amount, tx_id)
        }).collect() })
    }

    async fn execute_request_step(
        &mut self,
        request_id: MinterOperationId,
        request: RuneDepositPayload,
    ) -> ControlFlow<(), ()> {
        match request.status.clone() {
            DepositRequestStatus::Scheduled
            | DepositRequestStatus::WaitingForInputs { .. }
            | DepositRequestStatus::WaitingForConfirmations { .. } => {
                self.prepare_mint_orders(request_id, request).await
            }
            DepositRequestStatus::NothingToDeposit { .. } => ControlFlow::Break(()),
            DepositRequestStatus::InvalidAmounts { .. } => ControlFlow::Break(()),
            DepositRequestStatus::MintOrdersCreated { orders } => {
                let mut updated = vec![];
                let mut has_changes = false;
                for order_info in orders {
                    let MintOrderDetails {
                        rune_name,
                        amount,
                        status,
                    } = order_info;
                    if let MintOrderStatus::Created { mint_order, nonce } = status {
                        if let Ok(tx_id) = self.send_mint_order(&mint_order).await {
                            updated.push(MintOrderDetails {
                                status: MintOrderStatus::Sent {
                                    mint_order,
                                    tx_id,
                                    nonce,
                                },
                                rune_name,
                                amount,
                            });
                            has_changes = true;

                            continue;
                        }
                    }

                    updated.push(MintOrderDetails {
                        rune_name,
                        amount,
                        status,
                    });
                }

                if has_changes {
                    self.update_request_status(
                        request_id,
                        request,
                        DepositRequestStatus::MintOrdersCreated { orders: updated },
                    );
                }

                ControlFlow::Break(())
            }
            DepositRequestStatus::Minted { .. } => ControlFlow::Break(()),
            DepositRequestStatus::InternalError { .. } => ControlFlow::Break(()),
        }
    }

    async fn prepare_mint_orders(
        &mut self,
        request_id: MinterOperationId,
        request: RuneDepositPayload,
    ) -> ControlFlow<(), ()> {
        log::trace!("Preparing mint orders for operation {request_id}");

        let dst_address = &request.dst_address;
        let transit_address = self.get_transit_address(dst_address).await;

        let utxos_response = match self.get_deposit_utxos(&transit_address).await {
            Ok(utxos_response) if utxos_response.utxos.is_empty() => {
                self.wait_for_inputs(
                    request_id,
                    DepositRequestStatus::NothingToDeposit {
                        block_height: utxos_response.tip_height,
                    },
                );

                return ControlFlow::Break(());
            }
            Ok(utxos_response) => utxos_response,
            Err(err) => {
                self.wait_for_inputs(
                    request_id,
                    DepositRequestStatus::InternalError {
                        details: format!("{err:?}"),
                    },
                );
                return ControlFlow::Break(());
            }
        };

        match self.validate_utxo_confirmations(&utxos_response) {
            Ok(_) => {}
            Err(min_confirmations) => {
                self.wait_for_confirmations(request_id, utxos_response, min_confirmations);
                return ControlFlow::Break(());
            }
        }

        let utxos = utxos_response.utxos;

        let (rune_info_amounts, used_utxos) = match self
            .get_mint_amounts(&utxos, &request.requested_amounts)
            .await
        {
            Ok((amounts, _)) if amounts.is_empty() => {
                log::trace!("No runes found in the input utxos for request {request_id}.");

                self.wait_for_inputs(
                    request_id,
                    DepositRequestStatus::NothingToDeposit {
                        block_height: utxos_response.tip_height,
                    },
                );
                return ControlFlow::Break(());
            }
            Ok(v) => v,
            Err(err) => {
                self.wait_for_inputs(
                    request_id,
                    DepositRequestStatus::InternalError {
                        details: format!("{err:?}"),
                    },
                );
                return ControlFlow::Break(());
            }
        };

        log::trace!(
            "Found runes {:?} in utxos: {:?}",
            rune_info_amounts,
            used_utxos
        );

        if self.has_used_utxos(&used_utxos) {
            self.wait_for_inputs(
                request_id,
                DepositRequestStatus::InternalError {
                    details: "Utxos were consumed concurrently.".to_string(),
                },
            );
            return ControlFlow::Break(());
        }

        let mint_order_details = match self
            .create_mint_orders(
                &request.dst_address,
                &request.erc20_address,
                &rune_info_amounts,
            )
            .await
        {
            Ok(v) => v,
            Err(err) => {
                self.wait_for_inputs(
                    request_id,
                    DepositRequestStatus::InternalError {
                        details: format!("Failed to create a mint order: {err:?}"),
                    },
                );
                return ControlFlow::Break(());
            }
        };

        self.update_request_status(
            request_id,
            request,
            DepositRequestStatus::MintOrdersCreated {
                orders: mint_order_details,
            },
        );
        self.mark_used_utxos(&used_utxos, &transit_address);

        ControlFlow::Continue(())
    }

    fn wait_for_inputs(
        &mut self,
        request_id: MinterOperationId,
        bail_status: DepositRequestStatus,
    ) {
        let Some(request) = self.operation_store.get(request_id) else {
            log::error!("Deposit request {request_id} was unexpectedly removed from the store.");
            return;
        };

        let OperationState::Deposit(payload) = request else {
            log::error!("Request {request_id} is found but is not a deposit request.");
            return;
        };

        if self.is_request_timed_out(&payload) {
            self.update_request_status(request_id, payload, bail_status);
        } else {
            let block_height =
                if let DepositRequestStatus::NothingToDeposit { block_height } = bail_status {
                    block_height
                } else if let DepositRequestStatus::WaitingForInputs { block_height, .. } =
                    payload.status
                {
                    block_height
                } else {
                    0
                };

            self.update_request_status(
                request_id,
                payload.clone(),
                DepositRequestStatus::WaitingForInputs {
                    requested_at: payload.request_ts,
                    current_ts: ic::time(),
                    next_retry_at: ic::time() + self.deposit_retry_interval().as_nanos() as u64,
                    waiting_until: payload.request_ts + self.request_timeout().as_nanos() as u64,
                    block_height,
                },
            );

            self.reschedule_request(request_id);
        }
    }

    fn is_request_timed_out(&self, request: &RuneDepositPayload) -> bool {
        ic::time() > request.request_ts + self.request_timeout().as_nanos() as u64
    }

    fn request_timeout(&self) -> Duration {
        const REQUEST_TIME_OUT: Duration = Duration::from_secs(60 * 60 * 24);
        REQUEST_TIME_OUT
    }

    fn deposit_retry_interval(&self) -> Duration {
        const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 5);
        RETRY_INTERVAL
    }

    fn min_confirmations(&self) -> u32 {
        self.state.borrow().min_confirmations()
    }

    fn wait_for_confirmations(
        &mut self,
        request_id: MinterOperationId,
        utxos_response: GetUtxosResponse,
        current_min_confirmations: u32,
    ) {
        let Some(request) = self.operation_store.get(request_id) else {
            log::error!("Deposit request {request_id} was unexpectedly removed from the store.");
            return;
        };

        let OperationState::Deposit(payload) = request else {
            log::error!("Request {request_id} is found but is not a deposit request.");
            return;
        };

        self.update_request_status(
            request_id,
            payload,
            DepositRequestStatus::WaitingForConfirmations {
                utxos: utxos_response.utxos,
                current_min_confirmations,
                required_confirmations: self.min_confirmations(),
                block_height: utxos_response.tip_height,
            },
        );

        self.reschedule_request(request_id);
    }

    fn reschedule_request(&self, request_id: MinterOperationId) {
        self.scheduler.borrow_mut().append_task(
            RuneBridgeTask::Deposit(request_id).into_scheduled(
                TaskOptions::new()
                    .with_max_retries_policy(1)
                    .with_fixed_backoff_policy(self.deposit_retry_interval().as_secs() as u32),
            ),
        );
    }

    fn get_nonce(&self) -> u32 {
        NONCE.fetch_add(1, Ordering::Relaxed)
    }

    fn update_request_status(
        &mut self,
        request_id: MinterOperationId,
        request: RuneDepositPayload,
        new_status: DepositRequestStatus,
    ) {
        let updated_request = request.with_status(new_status);
        log::trace!("Changing status of deposit request: {request_id}: {updated_request:?}");
        self.operation_store
            .update(request_id, OperationState::Deposit(updated_request));
    }

    pub async fn get_deposit_utxos(
        &self,
        transit_address: &Address,
    ) -> Result<GetUtxosResponse, DepositError> {
        let mut utxo_response = self.utxo_provider.get_utxos(transit_address).await?;

        log::trace!(
            "Found {} utxos at address {transit_address}: {:?}.",
            utxo_response.utxos.len(),
            utxo_response.utxos
        );

        self.filter_out_used_utxos(&mut utxo_response);

        log::trace!(
            "Utxos at address {transit_address} after filtering out used utxos: {:?}",
            utxo_response.utxos
        );

        Ok(utxo_response)
    }

    async fn get_transit_address(&self, eth_address: &H160) -> Address {
        self.signer
            .get_transit_address(eth_address, self.network)
            .await
    }

    fn validate_utxo_confirmations(&self, utxo_info: &GetUtxosResponse) -> Result<(), u32> {
        let min_confirmations = self.state.borrow().min_confirmations();
        let utxo_min_confirmations = utxo_info
            .utxos
            .iter()
            .map(|utxo| utxo_info.tip_height - utxo.height + 1)
            .min()
            .unwrap_or_default();

        if min_confirmations > utxo_min_confirmations {
            Err(utxo_min_confirmations)
        } else {
            log::trace!(
                "Current utxo confirmations {} satisfies minimum {}. Proceeding.",
                utxo_min_confirmations,
                min_confirmations
            );
            Ok(())
        }
    }

    async fn fill_rune_infos(
        &self,
        rune_amounts: &HashMap<RuneName, u128>,
    ) -> Option<Vec<(RuneInfo, u128)>> {
        match self.fill_rune_infos_from_state(rune_amounts) {
            Some(v) => Some(v),
            None => self.fill_rune_infos_from_indexer(rune_amounts).await,
        }
    }

    fn fill_rune_infos_from_state(
        &self,
        rune_amounts: &HashMap<RuneName, u128>,
    ) -> Option<Vec<(RuneInfo, u128)>> {
        let state = self.state.borrow();
        let runes = state.runes();
        let mut infos = vec![];
        for (rune_name, amount) in rune_amounts {
            infos.push((*runes.get(rune_name)?, *amount));
        }

        Some(infos)
    }

    async fn fill_rune_infos_from_indexer(
        &self,
        rune_amounts: &HashMap<RuneName, u128>,
    ) -> Option<Vec<(RuneInfo, u128)>> {
        let rune_list = self.index_provider.get_rune_list().await.ok()?;
        let runes: HashMap<RuneName, RuneInfo> = rune_list
            .iter()
            .map(|(rune_id, spaced_rune, decimals)| {
                (
                    spaced_rune.rune.into(),
                    RuneInfo {
                        name: spaced_rune.rune.into(),
                        decimals: *decimals,
                        block: rune_id.block,
                        tx: rune_id.tx,
                    },
                )
            })
            .collect();
        let mut infos = vec![];
        for (rune_name, amount) in rune_amounts {
            match runes.get(rune_name) {
                Some(v) => infos.push((*v, *amount)),
                None => {
                    log::error!("Ord indexer didn't return a rune information for rune {rune_name} that was present in an UTXO");
                    return None;
                }
            }
        }

        self.state.borrow_mut().update_rune_list(runes);

        Some(infos)
    }

    async fn create_mint_order(
        &self,
        eth_address: &H160,
        erc20_address: &H160,
        amount: u128,
        rune_info: RuneInfo,
        nonce: u32,
    ) -> Result<SignedMintOrder, DepositError> {
        log::trace!("preparing mint order");

        let (signer, mint_order) = {
            let state_ref = self.state.borrow();

            let sender_chain_id = state_ref.btc_chain_id();
            let sender = Id256::from_evm_address(eth_address, sender_chain_id);
            let src_token = Id256::from(rune_info.id());

            let recipient_chain_id = state_ref.erc20_chain_id();

            let mint_order = MintOrder {
                amount: amount.into(),
                sender,
                src_token,
                recipient: eth_address.clone(),
                dst_token: erc20_address.clone(),
                nonce,
                sender_chain_id,
                recipient_chain_id,
                name: rune_info.name_array(),
                symbol: rune_info.symbol_array(),
                decimals: rune_info.decimals(),
                approve_spender: Default::default(),
                approve_amount: Default::default(),
                fee_payer: H160::default(),
            };

            let signer = state_ref.signer().get().clone();

            (signer, mint_order)
        };

        let signed_mint_order = mint_order
            .encode_and_sign(&signer)
            .await
            .map_err(|err| DepositError::Sign(format!("{err:?}")))?;

        Ok(signed_mint_order)
    }

    async fn create_mint_orders(
        &self,
        eth_address: &H160,
        erc20_address: &H160,
        rune_amounts: &[(RuneInfo, u128)],
    ) -> Result<Vec<MintOrderDetails>, DepositError> {
        let mut result = vec![];
        for (rune_info, amount) in rune_amounts {
            let nonce = self.get_nonce();
            let mint_order = self
                .create_mint_order(eth_address, erc20_address, *amount, *rune_info, nonce)
                .await?;
            result.push(MintOrderDetails {
                rune_name: rune_info.name,
                amount: *amount,
                status: MintOrderStatus::Created { mint_order, nonce },
            });
        }

        Ok(result)
    }

    async fn send_mint_order(&self, mint_order: &SignedMintOrder) -> Result<H256, DepositError> {
        log::trace!("Sending mint transaction");

        let signer = self.state.borrow().signer().get().clone();
        let sender = signer
            .get_address()
            .await
            .map_err(|err| DepositError::Sign(format!("{err:?}")))?;

        let (evm_info, evm_params) = {
            let state = self.state.borrow();

            let evm_info = state.get_evm_info();
            let evm_params = state
                .get_evm_params()
                .clone()
                .ok_or(DepositError::NotInitialized)?;

            (evm_info, evm_params)
        };

        let mut tx = minter_contract_utils::bft_bridge_api::mint_transaction(
            sender.0,
            evm_info.bridge_contract.0,
            evm_params.nonce.into(),
            evm_params.gas_price.into(),
            &mint_order.to_vec(),
            evm_params.chain_id as _,
        );

        let signature = signer
            .sign_transaction(&(&tx).into())
            .await
            .map_err(|err| DepositError::Sign(format!("{err:?}")))?;

        tx.r = signature.r.0;
        tx.s = signature.s.0;
        tx.v = signature.v.0;
        tx.hash = tx.hash();

        let client = evm_info.link.get_json_rpc_client();
        let id = client
            .send_raw_transaction(tx)
            .await
            .map_err(|err| DepositError::Evm(format!("{err:?}")))?;

        self.state.borrow_mut().update_evm_params(|p| {
            if let Some(params) = p.as_mut() {
                params.nonce += 1;
            }
        });

        log::trace!("Mint transaction sent");

        Ok(id.into())
    }

    fn filter_out_used_utxos(&self, get_utxos_response: &mut GetUtxosResponse) {
        let (_, existing) = self.state.borrow().ledger().load_unspent_utxos();

        get_utxos_response.utxos.retain(|utxo| {
            !existing.iter().any(|v| {
                v.outpoint.txid.as_byte_array()[..] == utxo.outpoint.txid
                    && v.outpoint.vout == utxo.outpoint.vout
            })
        })
    }

    fn has_used_utxos(&self, utxos: &[Utxo]) -> bool {
        let (_, existing) = self.state.borrow().ledger().load_unspent_utxos();

        utxos.iter().any(|utxo| {
            existing.iter().any(|v| {
                v.outpoint.txid.as_byte_array()[..] == utxo.outpoint.txid
                    && v.outpoint.vout == utxo.outpoint.vout
            })
        })
    }

    fn mark_used_utxos(&self, utxos: &[Utxo], address: &Address) {
        let mut state = self.state.borrow_mut();
        let ledger = state.ledger_mut();
        for utxo in utxos {
            ledger.mark_as_used((&utxo.outpoint).into(), address.clone());
        }
    }

    pub async fn get_mint_amounts(
        &self,
        utxos: &[Utxo],
        requested_amounts: &Option<HashMap<RuneName, u128>>,
    ) -> Result<(Vec<(RuneInfo, u128)>, Vec<Utxo>), DepositError> {
        let mut rune_amounts = HashMap::new();
        let mut used_utxos = vec![];

        for utxo in utxos {
            let tx_rune_amounts = self.index_provider.get_rune_amounts(utxo).await?;
            if !tx_rune_amounts.is_empty() {
                used_utxos.push(utxo.clone());
                for (rune_name, amount) in tx_rune_amounts {
                    *rune_amounts.entry(rune_name).or_default() += amount;
                }
            }
        }

        if rune_amounts.is_empty() {
            return Err(DepositError::NoRunesToDeposit);
        }

        if let Some(requested) = requested_amounts {
            if rune_amounts != *requested {
                return Err(DepositError::InvalidAmounts {
                    requested: requested.clone(),
                    actual: rune_amounts,
                });
            }
        }

        let Some(rune_info_amounts) = self.fill_rune_infos(&rune_amounts).await else {
            return Err(DepositError::Unavailable(
                "Ord indexer is in invalid state".to_string(),
            ));
        };

        Ok((rune_info_amounts, used_utxos))
    }
}
