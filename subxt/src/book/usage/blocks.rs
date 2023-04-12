// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

/*!
# Blocks

The [blocks API](crate::blocks::BlocksClient) in Subxt unifies many of the other interfaces, and allows you to:

- Access information about specific blocks (see [`crate::blocks::BlocksClient::at()`] and [`crate::blocks::BlocksClient::at_latest()`]).
- Subscribe to [all](crate::blocks::BlocksClient::subscribe_all()), [best](crate::blocks::BlocksClient::subscribe_best()) or [finalized](crate::blocks::BlocksClient::subscribe_finalized()) blocks as they are produced. Prefer to subscribe to finalized blocks unless you know what you're doing.

In either case, you'll end up with [`crate::blocks::Block`]'s, from which you can access various information about the block, such a the [header](crate::blocks::Block::header()), [block number](crate::blocks::Block::number()) and [body](crate::blocks::Block::body()). It also provides shortcuts to other Subxt APIs that work at a given block:

- [storage](crate::blocks::Block::storage()),
- [events](crate::blocks::Block::events())
- [runtime APIs](crate::blocks::Block::runtime_api())

Taken together, this means that you can subsscribe to blocks and then easily make use of the other Subxt APIs at each block.

Given a block, you can also [download the block body](crate::blocks::Block::body()) and iterate over the extrinsics stored within it using [`crate::blocks::BlockBody::extrinsics()`].

## Example

To put this together, here's an example of subscribing to blocks and printing a bunch of information about each one:

*/
//! ```rust,ignore
#![doc = include_str!("../../../../examples/examples/blocks_subscribing.rs")]
//! ```
/*!

*/
