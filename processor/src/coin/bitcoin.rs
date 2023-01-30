use async_trait::async_trait;
use bitcoin_hashes::hex::{FromHex, ToHex};
use std::str::FromStr;
use transcript::RecommendedTranscript;

use frost::{curve::Secp256k1, ThresholdKeys};

use bitcoin::{
  Block, OutPoint,
  schnorr::TweakedPublicKey,
  util::address::Address,
  consensus::encode,
  XOnlyPublicKey, SchnorrSighashType,
  psbt::{PartiallySignedTransaction, PsbtSighashType},
};

use bitcoin_serai::{
  rpc::Rpc,
  crypto::make_even,
  wallet::SpendableOutput,
  transactions::{TransactionMachine, SignableTransaction as BSignableTransaction},
};

use k256::{
  ProjectivePoint, Scalar,
  elliptic_curve::sec1::{ToEncodedPoint, Tag},
};
use crate::{
  coin::{CoinError, Block as BlockTrait, OutputType, Output as OutputTrait, Coin},
};

impl BlockTrait for Block {
  type Id = [u8; 32];
  fn id(&self) -> Self::Id {
    self.block_hash().as_ref().try_into().unwrap()
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fee {
  pub per_weight: u64,
}

impl Fee {
  pub fn calculate(&self, weight: usize) -> u64 {
    (self.per_weight * u64::try_from(weight).unwrap()) - 1
  }
}

#[derive(Clone, Debug)]
pub struct Output(SpendableOutput);
impl From<SpendableOutput> for Output {
  fn from(output: SpendableOutput) -> Output {
    Output(output)
  }
}
impl OutputTrait for Output {
  type Id = [u8; 36];

  //TODO: Implement later
  fn kind(&self) -> OutputType {
    OutputType::External
  }

  fn id(&self) -> Self::Id {
    encode::serialize(&self.0.output).try_into().unwrap()
  }

  fn amount(&self) -> u64 {
    self.0.amount
  }

  fn serialize(&self) -> Vec<u8> {
    self.0.serialize()
  }

  fn read<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
    SpendableOutput::read(reader).map(Output)
  }
}

#[derive(Debug)]
pub struct SignableTransaction {
  keys: ThresholdKeys<Secp256k1>,
  transcript: RecommendedTranscript,
  number: usize,
  actual: BSignableTransaction,
}

#[derive(Clone, Debug)]
pub struct Bitcoin {
  pub(crate) rpc: Rpc,
}
impl Bitcoin {
  pub async fn new(url: String) -> Bitcoin {
    Bitcoin { rpc: Rpc::new(url).unwrap() }
  }

  #[cfg(test)]
  fn test_address_with_key() -> (bitcoin::PrivateKey, bitcoin::PublicKey, Address) {
    use bitcoin::{PrivateKey, PublicKey, Network};
    use secp256k1::{rand, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::new(&mut rand::thread_rng());
    let private_key = PrivateKey::new(secret_key, Network::Regtest);
    let public_key = PublicKey::from_private_key(&secp, &private_key);
    let address = Address::p2pkh(&public_key, Network::Regtest);
    (private_key, public_key, address)
  }

  #[cfg(test)]
  fn test_get_spendables(block: &Block, address: &Address) -> Vec<SpendableOutput> {
    let mut outputs = Vec::new();
    for one_tx in &block.txdata {
      for (index, output_tx) in one_tx.output.iter().enumerate() {
        if output_tx.script_pubkey == address.script_pubkey() {
          outputs.push(SpendableOutput {
            output: OutPoint { txid: one_tx.txid(), vout: u32::try_from(index).unwrap() },
            amount: output_tx.value,
          });
        }
      }
    }
    return outputs;
  }
}

#[async_trait]
impl Coin for Bitcoin {
  type Curve = Secp256k1;

  type Fee = Fee;
  type Transaction = PartiallySignedTransaction;
  type Block = Block;

  type Output = Output;
  type SignableTransaction = SignableTransaction;
  type TransactionMachine = TransactionMachine;

  type Address = bitcoin::util::address::Address;

  const ID: &'static [u8] = b"Bitcoin";
  const CONFIRMATIONS: usize = 3;

  // TODO: Get hard numbers and tune
  const MAX_INPUTS: usize = 128;
  const MAX_OUTPUTS: usize = 16;

  fn address(&self, key: ProjectivePoint) -> Self::Address {
    debug_assert!(key.to_encoded_point(true).tag() == Tag::CompressedEvenY, "YKey is odd");
    let xonly_pubkey =
      XOnlyPublicKey::from_slice(&key.to_encoded_point(true).x().to_owned().unwrap()).unwrap();
    let tweaked_pubkey = TweakedPublicKey::dangerous_assume_tweaked(xonly_pubkey);
    Address::p2tr_tweaked(tweaked_pubkey, bitcoin::Network::Regtest)
  }

  //TODO: Implement later
  fn branch_address(&self, key: ProjectivePoint) -> Self::Address {
    self.address(key)
  }

  async fn get_latest_block_number(&self) -> Result<usize, CoinError> {
    Ok(self.rpc.get_latest_block_number().await.map_err(|_| CoinError::ConnectionError)?)
  }

  async fn get_block(&self, number: usize) -> Result<Self::Block, CoinError> {
    let block_hash = self.rpc.get_block_hash(number).await.unwrap();
    let info = self.rpc.get_block(&block_hash).await.unwrap();
    Ok(info)
  }

  async fn get_outputs(
    &self,
    block: &Self::Block,
    key: ProjectivePoint,
  ) -> Result<Vec<Self::Output>, CoinError> {
    let main_addr = self.address(key);
    let block_details = self.rpc.get_block(&block.block_hash()).await.unwrap();
    let mut outputs = Vec::new();
    for one_transaction in block_details.txdata {
      for (index, output_tx) in one_transaction.output.iter().enumerate() {
        if output_tx.script_pubkey == main_addr.script_pubkey() {
          outputs.push(Output(SpendableOutput {
            output: OutPoint { txid: one_transaction.txid(), vout: u32::try_from(index).unwrap() },
            amount: output_tx.value,
          }));
        }
      }
    }
    return Ok(outputs);
  }

  async fn prepare_send(
    &self,
    keys: ThresholdKeys<Secp256k1>,
    transcript: RecommendedTranscript,
    block_number: usize,
    mut inputs: Vec<Output>,
    payments: &[(Address, u64)],
    change: Option<ProjectivePoint>,
    fee: Fee,
  ) -> Result<Self::SignableTransaction, CoinError> {
    let mut vin_alt_list = Vec::new();
    let mut vout_alt_list = Vec::new();
    let change_addr = self.address(change.unwrap());

    let mut input_sat = 0;
    for one_input in &inputs {
      input_sat += one_input.amount();
      vin_alt_list.push(bitcoin::blockdata::transaction::TxIn {
        previous_output: one_input.0.output,
        script_sig: bitcoin::Script::new(),
        sequence: bitcoin::Sequence(u32::MAX),
        witness: bitcoin::Witness::default(),
      });
    }

    let mut payment_sat = 0;
    for one_payment in payments {
      payment_sat += one_payment.1;
      vout_alt_list.push(bitcoin::TxOut {
        value: one_payment.1,
        script_pubkey: one_payment.0.script_pubkey(),
      });
    }

    let mut actual_fee = fee
      .calculate(BSignableTransaction::calculate_weight(vin_alt_list.len(), &payments, false) * 2);
    if payment_sat > input_sat - actual_fee {
      return Err(CoinError::NotEnoughFunds);
    } else if input_sat != payment_sat {
      actual_fee = fee
        .calculate(BSignableTransaction::calculate_weight(vin_alt_list.len(), &payments, true) * 2);
      //TODO: we need to drop outputs worth less than payment_sat
      if payment_sat < (input_sat - actual_fee) {
        let rest_sat = input_sat - actual_fee - payment_sat;
        vout_alt_list
          .push(bitcoin::TxOut { value: rest_sat, script_pubkey: change_addr.script_pubkey() });
      }
    }

    let new_transaction = bitcoin::blockdata::transaction::Transaction {
      version: 2,
      lock_time: bitcoin::PackedLockTime(0),
      input: vin_alt_list,
      output: vout_alt_list,
    };
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(new_transaction.clone()).unwrap();
    for (i, one_input) in (&inputs).iter().enumerate() {
      let one_transaction =
        self.rpc.get_transaction(&one_input.0.output.txid, None, None).await.unwrap();
      let xonly_pubkey = XOnlyPublicKey::from_slice(
        &keys.group_key().to_encoded_point(true).x().to_owned().unwrap(),
      )
      .unwrap();
      psbt.inputs[i].witness_utxo =
        Some(one_transaction.output[usize::try_from(one_input.0.output.vout).unwrap()].clone());
      psbt.inputs[i].sighash_type = Some(PsbtSighashType::from(SchnorrSighashType::All));
      psbt.inputs[i].tap_internal_key = Some(xonly_pubkey);
    }
    return Ok(SignableTransaction {
      keys: keys,
      transcript: transcript,
      number: block_number + 1,
      actual: BSignableTransaction { tx: psbt, fee: actual_fee },
    });
  }

  async fn attempt_send(
    &self,
    transaction: Self::SignableTransaction,
  ) -> Result<Self::TransactionMachine, CoinError> {
    transaction
      .actual
      .clone()
      .multisig(transaction.keys.clone(), transaction.transcript.clone(), transaction.number)
      .await
      .map_err(|_| CoinError::ConnectionError)
  }

  async fn publish_transaction(
    &self,
    tx: &Self::Transaction,
  ) -> Result<(Vec<u8>, Vec<<Self::Output as OutputTrait>::Id>), CoinError> {
    let target_tx = tx.clone().extract_tx();
    let s_raw_transaction = self.rpc.send_raw_transaction(&target_tx).await.unwrap();
    let vec_output = target_tx
      .output
      .iter()
      .enumerate()
      .map(|(i, output)| {
        let one_output = SpendableOutput {
          output: OutPoint { txid: target_tx.txid(), vout: u32::try_from(i).unwrap() },
          amount: output.value,
        };
        Output(one_output).id()
      })
      .collect();
    Ok((s_raw_transaction.to_vec(), vec_output))
  }

  fn tweak_keys(&self, key: &mut ThresholdKeys<Self::Curve>) {
    if key.group_key().to_encoded_point(true).tag() == Tag::CompressedEvenY {
      return;
    }
    let (_, offset) = make_even(key.group_key());
    *key = key.offset(Scalar::from(offset));
  }

  #[cfg(test)]
  async fn get_fee(&self) -> Self::Fee {
    //TODO: Add fee estimation (42 satoshi / byte)
    Self::Fee { per_weight: 11 }
  }

  #[cfg(test)]
  async fn mine_block(&self) {
    use bitcoin::{
      Address, PrivateKey, PublicKey, Network,
      secp256k1::{SecretKey, rand, Secp256k1},
    };

    let secp = Secp256k1::new();
    let secret_key = SecretKey::new(&mut rand::thread_rng());
    let private_key = PrivateKey::new(secret_key, Network::Regtest);
    let public_key = PublicKey::from_private_key(&secp, &private_key);

    let new_addr = Address::p2wpkh(&public_key, Network::Regtest).unwrap();
    self
      .rpc
      .rpc_call::<Vec<String>>(
        "generatetoaddress".to_string(),
        &[1.into(), serde_json::to_value(&new_addr.to_string()).unwrap()],
      )
      .await
      .unwrap();
  }

  #[cfg(test)]
  async fn test_send(&self, address: Self::Address) {
    use bitcoin::{
      OutPoint, Sequence, Witness, Script, PackedLockTime,
      blockdata::{
        script::Builder,
        transaction::{TxIn, TxOut, Transaction},
      },
      secp256k1::{Secp256k1, Message},
    };

    let (private_key, public_key, main_addr) = Self::test_address_with_key();
    let new_block = self.get_latest_block_number().await.unwrap() + 1;
    self
      .rpc
      .rpc_call::<Vec<String>>(
        "generatetoaddress".to_string(),
        &[1.into(), serde_json::to_value(&main_addr.to_string()).unwrap()],
      )
      .await
      .unwrap();

    for _ in 0 .. 100 {
      self.mine_block().await;
    }

    let mut vin_list = Vec::new();
    let mut vout_list = Vec::new();
    let active_block = self.get_block(new_block).await.unwrap();
    let spendables = Self::test_get_spendables(&active_block, &main_addr);
    vin_list.push(TxIn {
      previous_output: OutPoint {
        txid: spendables[0].output.txid,
        vout: spendables[0].output.vout,
      },
      script_sig: Script::default(),
      sequence: Sequence(u32::MAX),
      witness: Witness::default(),
    });
    vout_list
      .push(TxOut { value: spendables[0].amount - 10000, script_pubkey: address.script_pubkey() });

    let mut new_transaction =
      Transaction { version: 2, lock_time: PackedLockTime(0), input: vin_list, output: vout_list };

    let secp = Secp256k1::new();
    let transactions_sighash = new_transaction.signature_hash(0, &main_addr.script_pubkey(), 1);
    let mut signed_der = secp
      .sign_ecdsa_low_r(&Message::from(transactions_sighash.as_hash()), &private_key.inner)
      .serialize_der()
      .to_vec();
    signed_der.push(1);
    new_transaction.input[0].script_sig =
      Builder::new().push_slice(&signed_der).push_key(&public_key).into_script();

    self.rpc.send_raw_transaction(&new_transaction).await.unwrap();
  }
}
