# Serai

Serai is a new DEX, built from the ground up, initially planning on listing
Bitcoin, Ethereum, Monero, and DAI, offering a liquidity-pool-based trading
experience. Funds are stored in an economically secured threshold-multisig
wallet.

[Getting Started](docs/Getting%20Started.md)

### Layout

- `docs`: Documentation on the Serai protocol.

- `common`: Crates containing utilities common to a variety of areas under
  Serai, none neatly fitting under another category.

- `crypto`: A series of composable cryptographic libraries built around the
  `ff`/`group` APIs achieving a variety of tasks. These range from generic
  infrastructure, to our IETF-compliant FROST implementation, to a DLEq proof as
  needed for Bitcoin-Monero atomic swaps.

- `coins`: Various coin libraries intended for usage in Serai yet also by the
  wider community. This means they will always support the functionality Serai
  needs, yet won't disadvantage other use cases when possible.

- `processor`: A generic chain processor to process data for Serai and process
  events from Serai, executing transactions as expected and needed.

- `substrate`: Substrate crates used to instantiate the Serai network.

- `deploy`: Scripts to deploy a Serai node/test environment.

### Links

- [Twitter](https://twitter.com/SeraiDEX):         https://twitter.com/SeraiDEX
- [Mastodon](https://cryptodon.lol/@serai):        https://cryptodon.lol/@serai
- [Discord](https://discord.gg/mpEUtJR3vz):        https://discord.gg/mpEUtJR3vz
- [Matrix](https://matrix.to/#/#serai:matrix.org):
https://matrix.to/#/#serai:matrix.org
