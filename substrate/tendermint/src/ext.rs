use core::{hash::Hash, fmt::Debug};
use std::sync::Arc;

use crate::Message;

pub trait ValidatorId: Send + Sync + Clone + Copy + PartialEq + Eq + Hash + Debug {}
impl<V: Send + Sync + Clone + Copy + PartialEq + Eq + Hash + Debug> ValidatorId for V {}

// Type aliases which are distinct according to the type system
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockNumber(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Round(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockError {
  // Invalid behavior entirely
  Fatal,
  // Potentially valid behavior dependent on unsynchronized state
  Temporal,
}

pub trait Block: Send + Sync + Clone + PartialEq + Debug {
  type Id: Send + Sync + Copy + Clone + PartialEq + Debug;

  fn id(&self) -> Self::Id;
}

pub trait Weights: Send + Sync {
  type ValidatorId: ValidatorId;

  fn total_weight(&self) -> u64;
  fn weight(&self, validator: Self::ValidatorId) -> u64;
  fn threshold(&self) -> u64 {
    ((self.total_weight() * 2) / 3) + 1
  }
  fn fault_thresold(&self) -> u64 {
    (self.total_weight() - self.threshold()) + 1
  }

  /// Weighted round robin function.
  fn proposer(&self, number: BlockNumber, round: Round) -> Self::ValidatorId;
}

#[async_trait::async_trait]
pub trait Network: Send + Sync {
  type ValidatorId: ValidatorId;
  type Weights: Weights<ValidatorId = Self::ValidatorId>;
  type Block: Block;

  // Block time in seconds
  const BLOCK_TIME: u32;

  fn weights(&self) -> Arc<Self::Weights>;

  async fn broadcast(&mut self, msg: Message<Self::ValidatorId, Self::Block>);

  // TODO: Should this take a verifiable reason?
  async fn slash(&mut self, validator: Self::ValidatorId);

  fn validate(&mut self, block: &Self::Block) -> Result<(), BlockError>;
  // Add a block and return the proposal for the next one
  fn add_block(&mut self, block: Self::Block) -> Self::Block;
}
