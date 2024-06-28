import { task } from 'hardhat/config';
import { HardhatRuntimeEnvironment } from 'hardhat/types';
import { Contract, ContractFactory, keccak256 } from 'ethers';
import { DeployImplementationResponse } from '@openzeppelin/hardhat-upgrades/dist/deploy-implementation';

interface UpgradeBftParams {
    proxyAddress: string;
    isWrapped: boolean;
    minterAddress: string;
    feeChargeAddress: string;
}

/**
   * Upgrades the BFT contract to a new implementation.
   *
   * @param proxyAddress - The address of the BFT proxy contract to be upgraded.
   * @param isWrapped - Whether the token is wrapped or not.
   * @param minterAddress - The address of the minter.
   * @param feeChargeAddress - The address of the fee charge address.
   * @param hre - The Hardhat Runtime Environment.
   * @returns The upgraded BFT contract.
   */
task('upgrade-bft', 'Upgrades the BFT contract')
    .addParam('proxyAddress', 'The address of the BFT proxy contract')
    .addParam('isWrapped', 'Whether the token is wrapped or not')
    .addParam('minterAddress', 'The address of the minter')
    .addParam('feeChargeAddress', 'The address of the fee charge address')
    .setAction(async (
        { proxyAddress, isWrapped, minterAddress, feeChargeAddress }: UpgradeBftParams,
        hre: HardhatRuntimeEnvironment
    ): Promise<void> => {
        try {
            console.log('Starting BFT contract upgrade process...');

            console.log('Compiling contract...');
            await hre.run('compile');
            console.log('Contract compiled successfully.');

            const { network } = hre.hardhatArguments;

            if (!network) {
                throw new Error('Network not specified. Please provide a network.');
            }

            function validateAddresses(...addresses: string[]) {
                addresses.forEach(address => {
                    if (!hre.ethers.isAddress(address)) {
                        throw new Error(`Invalid address: ${address}`);
                    }
                });
            }

            validateAddresses(proxyAddress, minterAddress, feeChargeAddress);


            console.log('Deploying new implementation contract...');

            /// Make use of versioned contract to deploy the new implementation contract
            const bftBridgeUpgrade = await hre.ethers.getContractFactory('BFTBridge');

            let bytecodeHash = keccak256(bftBridgeUpgrade.bytecode);

            /// Make sure you use the proxy contract address to get the
            /// contract instance
            /// and the old implementation contract should be the one that is currently deployed
            const proxyContract = await hre.ethers.getContractAt('BFTBridge', proxyAddress);

            console.log('Adding new implementation to the proxy contract...');
            let res = await proxyContract.addAllowedImplementation(bytecodeHash);

            let receipt = await res.wait();

            if (receipt!.status === 0) {
                throw new Error('Failed to add new implementation to the proxy contract.');
            }


            console.log('New implementation added successfully.');

            const newImplementationDeployment: DeployImplementationResponse = await hre.upgrades.prepareUpgrade(proxyAddress, bftBridgeUpgrade, {
                kind: 'uups',
                getTxResponse: false,
            });

            const newImplementationAddress: string = typeof newImplementationDeployment === 'string' ? newImplementationDeployment : await newImplementationDeployment.wait().then((tx) => tx?.contractAddress!);

            console.log(`New implementation contract deployed at: ${newImplementationAddress}`);

            console.log('Retrieving current proxy contract...');

            const initData: string = bftBridgeUpgrade.interface.encodeFunctionData('initialize', [minterAddress, feeChargeAddress, isWrapped]);

            console.log('Upgrading proxy contract to new implementation...');
            await proxyContract.upgradeToAndCall(newImplementationAddress, initData);
            console.log('Proxy contract upgraded successfully.');

            console.log('BFT contract upgrade completed.');
            console.log(`Proxy address: ${proxyAddress}`);
            console.log(`New implementation address: ${newImplementationAddress}`);
        } catch (error) {
            console.error('Error during BFT contract upgrade:', error);
            throw error;
        }
    });