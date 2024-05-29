// Copyright 2022-2024 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

//! Helper commands for interacting with the Delorean/CETF actor via RPC
//!
//! The example assumes that Tendermint and Fendermint have been started
//! and are running locally.
//!
//! # Usage
//! ```text
//! cargo run --example delorean -- --secret-key test-data/keys/volvo.sk queue-tag
//! ```

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use bls_signatures::Serialize;
use bytes::Bytes;
use clap::{Parser, Subcommand};
use ethers::abi::Tokenizable;
use ethers::prelude::*;
use fendermint_actor_cetf as cetf_actor;
use fendermint_rpc::query::QueryClient;
use fendermint_vm_actor_interface::eam::{self, CreateReturn, EthAddress};
use fendermint_vm_message::query::FvmQueryHeight;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::chainid::ChainID;
use lazy_static::lazy_static;
use tendermint_rpc::Url;
use tracing::Level;

use fvm_shared::econ::TokenAmount;

use fendermint_rpc::client::FendermintClient;
use fendermint_rpc::message::{GasParams, SignedMessageFactory};
use fendermint_rpc::tx::{CallClient, TxClient, TxCommit};

type MockProvider = ethers::providers::Provider<ethers::providers::MockProvider>;
type MockContractCall<T> = ethers::prelude::ContractCall<MockProvider, T>;

const EXAMPLE_CONTRACT_SPEC_JSON: &str =
    include_str!("../../../../contracts/out/Example.sol/CetfExample.json");

lazy_static! {
    /// Default gas params based on the testkit.
    static ref GAS_PARAMS: GasParams = GasParams {
        gas_limit: 10_000_000_000,
        gas_fee_cap: TokenAmount::default(),
        gas_premium: TokenAmount::default(),
    };
}

abigen!(
    CetfExample,
    r#"[
        {
            "type": "function",
            "name": "releaseKey",
            "inputs": [
                {
                    "name": "tag",
                    "type": "uint64",
                    "internalType": "uint64"
                }
            ],
            "outputs": [
                {
                    "name": "",
                    "type": "int256",
                    "internalType": "int256"
                }
            ],
            "stateMutability": "nonpayable"
        },
        {
            "type": "error",
            "name": "ActorNotFound",
            "inputs": []
        },
        {
            "type": "error",
            "name": "FailToCallActor",
            "inputs": []
        }
    ]"#
);

#[derive(Parser, Debug)]
pub struct Options {
    /// The URL of the Tendermint node's RPC endpoint.
    #[arg(
        long,
        short,
        default_value = "http://127.0.0.1:26657",
        env = "TENDERMINT_RPC_URL"
    )]
    pub url: Url,

    /// Enable DEBUG logs.
    #[arg(long, short)]
    pub verbose: bool,

    #[command(subcommand)]
    command: Commands,

    /// Path to the secret key to deploy with, expected to be in Base64 format,
    /// and that it has a corresponding f410 account in genesis.
    #[arg(long, short)]
    pub secret_key: PathBuf,
}

#[derive(Debug, Subcommand)]
enum Commands {
    RegisterBls {
        #[arg(long, short)]
        bls_secret_key: PathBuf,
    },
    QueueTag,
    DeployExampleContract,
    CallExampleContract {
        address: String,
    },
}

impl Options {
    pub fn log_level(&self) -> Level {
        if self.verbose {
            Level::DEBUG
        } else {
            Level::INFO
        }
    }
}

/// See the module docs for how to run.
#[tokio::main]
async fn main() {
    let opts: Options = Options::parse();

    tracing_subscriber::fmt()
        .with_max_level(opts.log_level())
        .init();

    let client = FendermintClient::new_http(opts.url, None).expect("error creating client");

    let sk =
        SignedMessageFactory::read_secret_key(&opts.secret_key).expect("error reading secret key");

    let pk = sk.public_key();

    let f1_addr = Address::new_secp256k1(&pk.serialize()).expect("valid public key");

    // Query the account nonce from the state, so it doesn't need to be passed as an arg.
    let sn = sequence(&client, &f1_addr)
        .await
        .expect("error getting sequence");

    // Query the chain ID, so it doesn't need to be passed as an arg.
    let chain_id = client
        .state_params(FvmQueryHeight::default())
        .await
        .expect("error getting state params")
        .value
        .chain_id;

    let mf = SignedMessageFactory::new(sk, f1_addr, sn, ChainID::from(chain_id));

    let mut client = client.bind(mf);

    match opts.command {
        Commands::RegisterBls { bls_secret_key } => {
            let bls_sk = {
                let b64 = std::fs::read_to_string(&bls_secret_key)
                    .expect("failed to read bls secret key");
                bls_signatures::PrivateKey::from_bytes(
                    &fendermint_crypto::from_b64(&b64)
                        .expect("failed to decode b64 bls secret key"),
                )
                .expect("failed to parse bls secret key")
            };

            let bls_pk = bls_sk.public_key();

            let res = TxClient::<TxCommit>::transaction(
                &mut client,
                fendermint_vm_actor_interface::cetf::CETFSYSCALL_ACTOR_ADDR,
                cetf_actor::Method::AddValidator as u64,
                RawBytes::serialize(cetf_actor::AddValidatorParams {
                    address: f1_addr,
                    public_key: fendermint_actor_cetf::BlsPublicKey(
                        bls_pk
                            .as_bytes()
                            .try_into()
                            .expect("Failed to convert BLS public key to bytes"),
                    ),
                })
                .expect("failed to serialize params"),
                TokenAmount::from_whole(0),
                GAS_PARAMS.clone(),
            )
            .await
            .expect("transfer failed");

            assert!(res.response.check_tx.code.is_ok(), "check is ok");
            assert!(res.response.tx_result.code.is_ok(), "deliver is ok");
            assert!(res.return_data.is_some());
        }
        Commands::QueueTag => {

            let bytes = RawBytes::serialize(88_u64)
            .expect("failed to serialize params");
            tracing::info!("CBOR encoded input should look like: {:?}", bytes);

            let res = TxClient::<TxCommit>::transaction(
                &mut client,
                fendermint_vm_actor_interface::cetf::CETFSYSCALL_ACTOR_ADDR,
                cetf_actor::Method::EnqueueTag as u64,
                RawBytes::serialize(88_64)
                    .expect("failed to serialize params"),
                TokenAmount::from_whole(0),
                GAS_PARAMS.clone(),
            )
            .await
            .expect("transfer failed");

            assert!(res.response.check_tx.code.is_ok(), "check is ok");
            assert!(res.response.tx_result.code.is_ok(), "deliver is ok");
            assert!(res.return_data.is_some());
        }
        Commands::DeployExampleContract => {
            let spec: serde_json::Value =
                serde_json::from_str(EXAMPLE_CONTRACT_SPEC_JSON).expect("failed to parse contract spec");

            let example_contract = hex::decode(
                &spec["bytecode"]["object"]
                    .as_str()
                    .expect("missing bytecode")
                    .trim_start_matches("0x"),
            )
            .expect("invalid hex");

            tracing::info!("Deploying Example Contract");

            let res = TxClient::<TxCommit>::fevm_create(
                &mut client,
                Bytes::from(example_contract),
                Bytes::default(),
                TokenAmount::default(),
                GAS_PARAMS.clone(),
            )
            .await
            .expect("error deploying contract");

            tracing::info!(tx_hash = ?res.response.hash, "deployment transaction");

            let ret = res
                .return_data
                .ok_or(anyhow!(
                    "no CreateReturn data; response was {:?}",
                    res.response
                ))
                .expect("failed to get CreateReturn data");
            let address = ret.eth_address;
            tracing::info!(address = ?address, "contract deployed");

        }
        Commands::CallExampleContract { address } => {
            let contract = example_contract(&address);

            let call = contract.release_key(88);

            let result: I256 = invoke_or_call_contract(&mut client, &address, call, true)
                .await
                .expect("failed to call contract");

            tracing::info!(result = ?result, "contract call result");
        }
    }
}

/// Invoke FEVM through Tendermint with the calldata encoded by ethers, decoding the result into the expected type.
async fn invoke_or_call_contract<T: Tokenizable>(
    client: &mut (impl TxClient<TxCommit> + CallClient),
    contract_eth_addr: &str,
    call: MockContractCall<T>,
    in_transaction: bool,
) -> anyhow::Result<T> {
    let calldata: ethers::types::Bytes = call
        .calldata()
        .expect("calldata should contain function and parameters");

    let contract_addr = eth_addr_to_eam(contract_eth_addr);

    // We can perform the read as a distributed transaction (if we don't trust any particular node to give the right answer),
    // or we can send a query with the same message and get a result without involving a transaction.
    let return_data = if in_transaction {
        let res = client
            .fevm_invoke(
                contract_addr,
                calldata.0,
                TokenAmount::default(),
                GAS_PARAMS.clone(),
            )
            .await
            .context("failed to invoke FEVM")?;

        tracing::info!(tx_hash = ?res.response.hash, "invoked transaction");

        res.return_data
    } else {
        let res = client
            .fevm_call(
                contract_addr,
                calldata.0,
                TokenAmount::default(),
                GAS_PARAMS.clone(),
                FvmQueryHeight::default(),
            )
            .await
            .context("failed to call FEVM")?;

        res.return_data
    };

    let bytes = return_data.ok_or(anyhow!("the contract did not return any data"))?;

    let res = decode_function_data(&call.function, bytes, false)
        .context("error deserializing return data")?;

    Ok(res)
}

/// Get the next sequence number (nonce) of an account.
async fn sequence(client: &impl QueryClient, addr: &Address) -> anyhow::Result<u64> {
    let state = client
        .actor_state(&addr, FvmQueryHeight::default())
        .await
        .context("failed to get actor state")?;

    match state.value {
        Some((_id, state)) => Ok(state.sequence),
        None => Err(anyhow!("cannot find actor {addr}")),
    }
}

/// Create an instance of the statically typed contract client.
fn example_contract(contract_eth_addr: &str) -> CetfExample<MockProvider> {
    // A dummy client that we don't intend to use to call the contract or send transactions.
    let (client, _mock) = ethers::providers::Provider::mocked();
    let contract_h160_addr = ethers::core::types::Address::from_slice(
        hex::decode(contract_eth_addr.trim_start_matches("0x"))
            .unwrap()
            .as_slice(),
    );
    let contract = CetfExample::new(contract_h160_addr, std::sync::Arc::new(client));
    contract
}

fn eth_addr_to_eam(eth_addr: &str) -> Address {
    let eth_addr = hex::decode(eth_addr.trim_start_matches("0x")).expect("valid hex");
    Address::new_delegated(eam::EAM_ACTOR_ID, &eth_addr)
        .expect("ETH address to delegated should work")
}
