use std::sync::{Arc, RwLock};

use sp_core::Decode;
use sp_runtime::traits::{Hash, Header, Block};

use sc_network::PeerId;
use sc_network_gossip::{Validator, ValidatorContext, ValidationResult};

use tendermint_machine::{ext::SignatureScheme, SignedMessage};

use crate::{TendermintValidator, validators::TendermintValidators};

#[derive(Clone)]
pub(crate) struct TendermintGossip<T: TendermintValidator> {
  number: Arc<RwLock<u64>>,
  signature_scheme: Arc<TendermintValidators<T>>,
}

impl<T: TendermintValidator> TendermintGossip<T> {
  pub(crate) fn new(
    number: Arc<RwLock<u64>>,
    signature_scheme: Arc<TendermintValidators<T>>,
  ) -> Self {
    TendermintGossip { number, signature_scheme }
  }

  pub(crate) fn topic(number: u64) -> <T::Block as Block>::Hash {
    <<<T::Block as Block>::Header as Header>::Hashing as Hash>::hash(
      &[b"Tendermint Block Topic".as_ref(), &number.to_le_bytes()].concat(),
    )
  }
}

impl<T: TendermintValidator> Validator<T::Block> for TendermintGossip<T> {
  fn validate(
    &self,
    _: &mut dyn ValidatorContext<T::Block>,
    _: &PeerId,
    data: &[u8],
  ) -> ValidationResult<<T::Block as Block>::Hash> {
    let msg = match SignedMessage::<
      u16,
      T::Block,
      <TendermintValidators<T> as SignatureScheme>::Signature,
    >::decode(&mut &*data)
    {
      Ok(msg) => msg,
      Err(_) => return ValidationResult::Discard,
    };

    if msg.number().0 < *self.number.read().unwrap() {
      return ValidationResult::Discard;
    }

    if !msg.verify_signature(&self.signature_scheme) {
      return ValidationResult::Discard;
    }

    ValidationResult::ProcessAndKeep(Self::topic(msg.number().0))
  }
}