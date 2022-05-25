# finality-pbft

**Please keep in mind that this is a POC (Proof of Concept) work that should be refined further before being put into production.**

> pBFT (Practical Byzantine Fault Tolerance) is a consensus algorithm introduced in the late 90s by Barbara Liskov and Miguel Castro.

finality-pbft is a finality gadget for [substrate][substrate], written in Rust and uses `async/await`.
It allows a set of nodes to reach BFT
agreement on what is the canonical chain, which is produced by some external block production
mechanism. It works under the assumption of a partially synchronous network model and with the
presence of up to 1/3 Byzantine nodes.

In addition to origin pBFT, finality-pbft also supports the following features:

- catching up in current view.
- authority changes.
- quick catching up.

And there are other distincitions between pBFT and finality-pbft:

- view changing.
- catch up strategy.

This codebase takes a lot of inspiration from [finality-grandpa][substrate-finality-grandpa], and more thorough implementation considerations are discussed below.

## Build & Test

```
git clone https://github.com/fky2015/finality-pbft
cd finality-pbft
cargo build
cargo test
```

## Usage

Add this to your Cargo.toml:

```toml
[dependencies]
finality-pbft = "0.15"
```

**Features:**

- `derive-codec` - Derive `Decode`/`Encode` instances of [parity-scale-codec][parity-scale-codec]
  for all the protocol messages.
- `test-helpers` - Expose some opaque types for testing purposes.

### Integration

This crate only implements the state machine for the pBFT protocol. In order to use this crate it
is necessary to implement some traits to do the integration which are responsible for providing
access to the underyling blockchain and setting up all the network communication.

In a word, you should also refer to [substrate-pBFT][substrate-pBFT] to get a runnable substrate node.

#### [`Environment`][environment-docs]

The `Environment` trait defines the types that will be used for the input and output stream to
receive and broadcast messages. It is also responsible for setting these up for a given round
(through `round_data`).

The trait exposes callbacks for notifying about block finality.

### Substrate

The main user of this crate is [substrate-pBFT][substrate-pBFT] and should be the main resource used to look
into how the integration is done. The [`substrate-finality-pbft` crate][substrate-finality-pbft]
should have most of the relevant integration code.

Most importantly this crate does not deal with authority set changes. It assumes that the set of
authorities is always the same. Authority set handoffs are handled in Substrate by listening to
signals emitted on the underlying blockchain.

## Implementation Details

Below, We'll go through some implementation considerations. 
Before reading this, you should have a basic understanding of GRANDPA and pBFT.

### About Leader Election

GRANDPA does not required a leader, although pBFT does. However, because consensus is divided into two phases: block creation and block finalization, things are becoming more complicated in the substrate. (in particular, AURA and GRANDPA)

Before implementing finality-pbft, I asked [a question on Stack Overflow](https://stackoverflow.com/questions/69993877/is-it-feasible-to-implement-a-raft-pbft-like-leader-election-between-authorized). And, as a result of this study, I now can say that a leader-based consensus method can be implemented in substrate.

First, the finality gadget, such as finality-pbft, should be in charge of maintaining a leader, failing until election is made.

Then, this status should be synced with sc-pbft (substrate client) and exposed to sc-aura in further.

In AURA, we can utilize this state to decide whether or not the node should generate a new block.

This technique can be used to implement any leader-based method.

### Gossip Network

In finality-grandpa, gossip network is devided into two sections: the one for global and the rests (for rounds).

`sc-grandpa` and `libp2p` make sure that only nodes in the same round can discover each other. (A node can be in different rounds at the same time.)

GRANDPA defines a round as a single cycle from proposal to completion.

When comes into pBFT, however, things get a little bit more complicated. pBFT has a term called "view". A view may contains multiple rounds. The view number will change only when catch-up or primariy failure happens.

As a result, I mapped the concept of "view" in pBFT to "round" in grandpa, which means that nodes in the same view can communicate with each other. And a node can only be in one view at a time.

In finality-grandpa, global messages are `Commit` and `CatchUp`. `Commit` denotes a finalized block (or blocks) and `CatchUp` denotes catch-up.

In finality-pbft, A message for view change has been added in the global network. And round logic also handles block finalization.

### Async/Await instead of Futures

It's a wild decision that implementing finality-pbft using `async/await` when finality-grandpa is still using `Futures`.

In finality-grandpa, a node is made by a single `Future`. In a single poll, a struct can hold both incoming and outgoing (channels), and process incoming before making progress on consensus (outgoing). However, this hard-coded processing order leaves a potential vulnerability. When a node is under DDOS, it may always consuming incoming message and return early.

When use `async/await`, we can (and should) separate incoming and outgoing into two different async function, using `select!` to poll them "together", and let runtime decide the order. However, the trade-off is that a struct can no longer hold both incoming and outgoing. Ownership should be moved before sealing the `Future`.


### A Little bit of Mess in Early Git Commit

Despite my best efforts to keep the commit history clean and tidy, there are still a few messes near the start.

Because things aren't quite clear at the moment. I had to fumble around for a way out.

Sorry for the inconvenience :).

### Test

finality-grandpa test provide a basic test network that can connect multiple nodes. 

**Upon that, I tagged the sender and receiver of the messages and added a injectable rule controller so that we can simulate network failure in tests.**

For further info, please see the code.

### State Management

The term "state" refers to all of the data that determines the state of a state machine.

Since `async/await` need us to separate message sending and receiving into two concurrent function, and both functions will change the state. As a result, this storage struct must be wrapped with `Arc<Mutex<>>` and shared among async functions and instances.

## Resources

- [Paper][paper]

## License

Usage is provided under the Apache License (Version 2.0). See [LICENSE](LICENSE) for the full
details.

[substrate]: https://github.com/paritytech/substrate
[substrate-finality-grandpa]: https://github.com/paritytech/substrate/tree/master/client/finality-grandpa
[substrate-pBFT]: https://github.com/fky2015/substrate-pBFT
[paper]: https://pmg.csail.mit.edu/papers/osdi99.pdf
