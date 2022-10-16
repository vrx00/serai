use std::{sync::Arc, time::Instant, collections::HashMap};

use tokio::{
  task::{JoinHandle, yield_now},
  sync::{
    RwLock,
    mpsc::{self, error::TryRecvError},
  },
};

pub mod ext;
use ext::*;

mod message_log;
use message_log::MessageLog;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Step {
  Propose,
  Prevote,
  Precommit,
}

#[derive(Clone, PartialEq, Debug)]
enum Data<B: Block> {
  Proposal(Option<Round>, B),
  Prevote(Option<B::Id>),
  Precommit(Option<B::Id>),
}

impl<B: Block> Data<B> {
  fn step(&self) -> Step {
    match self {
      Data::Proposal(..) => Step::Propose,
      Data::Prevote(..) => Step::Prevote,
      Data::Precommit(..) => Step::Precommit,
    }
  }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Message<V: ValidatorId, B: Block> {
  sender: V,

  number: BlockNumber,
  round: Round,

  data: Data<B>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TendermintError<V: ValidatorId> {
  Malicious(V),
  Temporal,
}

pub struct TendermintMachine<N: Network> {
  network: Arc<RwLock<N>>,
  weights: Arc<N::Weights>,
  proposer: N::ValidatorId,

  number: BlockNumber,
  personal_proposal: N::Block,

  log: MessageLog<N>,
  round: Round,
  step: Step,

  locked: Option<(Round, N::Block)>,
  valid: Option<(Round, N::Block)>,

  timeouts: HashMap<Step, Instant>,
}

pub struct TendermintHandle<N: Network> {
  // Messages received
  pub messages: mpsc::Sender<Message<N::ValidatorId, N::Block>>,
  // Async task executing the machine
  pub handle: JoinHandle<()>,
}

impl<N: Network + 'static> TendermintMachine<N> {
  fn timeout(&self, step: Step) -> Instant {
    todo!()
  }

  #[async_recursion::async_recursion]
  async fn broadcast(&mut self, data: Data<N::Block>) -> Option<N::Block> {
    let msg = Message { sender: self.proposer, number: self.number, round: self.round, data };
    let res = self.message(msg.clone()).await.unwrap();
    self.network.write().await.broadcast(msg).await;
    res
  }

  // 14-21
  async fn round_propose(&mut self) {
    if self.weights.proposer(self.number, self.round) == self.proposer {
      let (round, block) = if let Some((round, block)) = &self.valid {
        (Some(*round), block.clone())
      } else {
        (None, self.personal_proposal.clone())
      };
      debug_assert!(self.broadcast(Data::Proposal(round, block)).await.is_none());
    } else {
      self.timeouts.insert(Step::Propose, self.timeout(Step::Propose));
    }
  }

  // 11-13
  async fn round(&mut self, round: Round) {
    self.round = round;
    self.step = Step::Propose;
    self.round_propose().await;
  }

  // 1-9
  async fn reset(&mut self, proposal: N::Block) {
    self.number.0 += 1;
    self.personal_proposal = proposal;

    self.log = MessageLog::new(self.network.read().await.weights());

    self.locked = None;
    self.valid = None;

    self.timeouts = HashMap::new();

    self.round(Round(0)).await;
  }

  // 10
  pub fn new(
    network: N,
    proposer: N::ValidatorId,
    number: BlockNumber,
    proposal: N::Block,
  ) -> TendermintHandle<N> {
    let (msg_send, mut msg_recv) = mpsc::channel(100); // Backlog to accept. Currently arbitrary
    TendermintHandle {
      messages: msg_send,
      handle: tokio::spawn(async move {
        let weights = network.weights();
        let network = Arc::new(RwLock::new(network));
        let mut machine = TendermintMachine {
          network,
          weights: weights.clone(),
          proposer,

          number,
          personal_proposal: proposal,

          log: MessageLog::new(weights),
          round: Round(0),
          step: Step::Propose,

          locked: None,
          valid: None,

          timeouts: HashMap::new(),
        };
        dbg!("Proposing");
        machine.round_propose().await;

        loop {
          // Check if any timeouts have been triggered
          let now = Instant::now();
          let (t1, t2, t3) = {
            let ready = |step| machine.timeouts.get(&step).unwrap_or(&now) < &now;
            (ready(Step::Propose), ready(Step::Prevote), ready(Step::Precommit))
          };

          // Propose timeout
          if t1 {
            todo!()
          }

          // Prevote timeout
          if t2 {
            todo!()
          }

          // Precommit timeout
          if t3 {
            todo!()
          }

          // If there's a message, handle it
          match msg_recv.try_recv() {
            Ok(msg) => match machine.message(msg).await {
              Ok(None) => (),
              Ok(Some(block)) => {
                let proposal = machine.network.write().await.add_block(block);
                machine.reset(proposal).await
              }
              Err(TendermintError::Malicious(validator)) => {
                machine.network.write().await.slash(validator).await
              }
              Err(TendermintError::Temporal) => (),
            },
            Err(TryRecvError::Empty) => yield_now().await,
            Err(TryRecvError::Disconnected) => break,
          }
        }
      }),
    }
  }

  // 49-54
  fn check_committed(&mut self, round: Round) -> Option<N::Block> {
    let proposer = self.weights.proposer(self.number, round);

    // Get the proposal
    if let Some(proposal) = self.log.get(round, proposer, Step::Propose) {
      // Destructure
      debug_assert!(matches!(proposal, Data::Proposal(..)));
      if let Data::Proposal(_, block) = proposal {
        // Check if it has gotten a sufficient amount of precommits
        let (participants, weight) =
          self.log.message_instances(round, Data::Precommit(Some(block.id())));

        let threshold = self.weights.threshold();
        if weight >= threshold {
          return Some(block.clone());
        }

        // 47-48
        if participants >= threshold {
          let timeout = self.timeout(Step::Precommit);
          self.timeouts.entry(Step::Precommit).or_insert(timeout);
        }
      }
    }

    None
  }

  async fn message(
    &mut self,
    msg: Message<N::ValidatorId, N::Block>,
  ) -> Result<Option<N::Block>, TendermintError<N::ValidatorId>> {
    if msg.number != self.number {
      Err(TendermintError::Temporal)?;
    }

    if matches!(msg.data, Data::Proposal(..)) &&
      (msg.sender != self.weights.proposer(msg.number, msg.round))
    {
      Err(TendermintError::Malicious(msg.sender))?;
    };

    if !self.log.log(msg.clone())? {
      return Ok(None);
    }

    // All functions, except for the finalizer and the jump, are locked to the current round
    // Run the finalizer to see if it applies
    if matches!(msg.data, Data::Proposal(..)) || matches!(msg.data, Data::Precommit(_)) {
      let block = self.check_committed(msg.round);
      if block.is_some() {
        return Ok(block);
      }
    }

    // Else, check if we need to jump ahead
    if msg.round.0 < self.round.0 {
      return Ok(None);
    } else if msg.round.0 > self.round.0 {
      // 55-56
      if self.log.round_participation(self.round) > self.weights.fault_thresold() {
        self.round(msg.round);
      } else {
        return Ok(None);
      }
    }

    let proposal = self
      .log
      .get(self.round, self.weights.proposer(self.number, self.round), Step::Propose)
      .cloned();
    if self.step == Step::Propose {
      if let Some(proposal) = &proposal {
        debug_assert!(matches!(proposal, Data::Proposal(..)));
        if let Data::Proposal(vr, block) = proposal {
          if let Some(vr) = vr {
            // 28-33
            if (vr.0 < self.round.0) && self.log.has_consensus(*vr, Data::Prevote(Some(block.id())))
            {
              debug_assert!(self
                .broadcast(Data::Prevote(Some(block.id()).filter(|_| {
                  self
                    .locked
                    .as_ref()
                    .map(|(round, value)| (round.0 <= vr.0) || (block.id() == value.id()))
                    .unwrap_or(true)
                })))
                .await
                .is_none());
              self.step = Step::Prevote;
            } else {
              Err(TendermintError::Malicious(msg.sender))?;
            }
          } else {
            // 22-27
            self
              .network
              .write()
              .await
              .validate(block)
              .map_err(|_| TendermintError::Malicious(msg.sender))?;
            debug_assert!(self
              .broadcast(Data::Prevote(Some(block.id()).filter(|_| self.locked.is_none() ||
                self.locked.as_ref().map(|locked| locked.1.id()) == Some(block.id()))))
              .await
              .is_none());
            self.step = Step::Prevote;
          }
        }
      }
    }

    if self.step == Step::Prevote {
      let (participation, weight) = self.log.message_instances(self.round, Data::Prevote(None));
      // 34-35
      if participation > self.weights.threshold() {
        let timeout = self.timeout(Step::Prevote);
        self.timeouts.entry(Step::Prevote).or_insert(timeout);
      }

      // 44-46
      if weight > self.weights.threshold() {
        debug_assert!(self.broadcast(Data::Precommit(None)).await.is_none());
        self.step = Step::Precommit;
      }
    }

    if (self.valid.is_none()) && ((self.step == Step::Prevote) || (self.step == Step::Precommit)) {
      if let Some(proposal) = proposal {
        debug_assert!(matches!(proposal, Data::Proposal(..)));
        if let Data::Proposal(_, block) = proposal {
          if self.log.has_consensus(self.round, Data::Prevote(Some(block.id()))) {
            self.valid = Some((self.round, block.clone()));
            if self.step == Step::Prevote {
              self.locked = self.valid.clone();
              self.step = Step::Precommit;
              return Ok(self.broadcast(Data::Precommit(Some(block.id()))).await);
            }
          }
        }
      }
    }

    Ok(None)
  }
}
