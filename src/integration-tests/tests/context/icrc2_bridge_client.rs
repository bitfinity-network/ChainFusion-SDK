use did::H160;
use ic_canister_client::{CanisterClient, CanisterClientResult};
use icrc2_minter::operation::OperationState;
use minter_contract_utils::operation_store::MinterOperationId;
use minter_did::id256::Id256;
use minter_did::order::SignedMintOrder;

use crate::context::bridge_client::BridgeCanisterClient;

pub struct Icrc2BridgeClient<C> {
    client: C,
}

impl<C: CanisterClient> Icrc2BridgeClient<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    pub async fn get_operations_list(
        &self,
        wallet_address: &H160,
        offset: Option<u64>,
        count: Option<u64>,
    ) -> CanisterClientResult<Vec<(MinterOperationId, OperationState)>> {
        self.client
            .query("get_operations_list", (wallet_address, offset, count))
            .await
    }

    pub async fn list_mint_orders(
        &self,
        wallet_address: &H160,
        src_token: &Id256,
        offset: Option<u64>,
        count: Option<u64>,
    ) -> CanisterClientResult<Vec<(u32, SignedMintOrder)>> {
        self.client
            .query(
                "list_mint_orders",
                (wallet_address, src_token, offset, count),
            )
            .await
    }
}

impl<C: CanisterClient> BridgeCanisterClient<C> for Icrc2BridgeClient<C> {
    fn client(&self) -> &C {
        &self.client
    }
}
