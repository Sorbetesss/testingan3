// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use crate::{test_context, utils::node_runtime};
use codec::{Compact, Encode};
use futures::StreamExt;
use subxt_metadata::Metadata;
use subxt_signer::sr25519::dev;

#[tokio::test]
async fn block_subscriptions_are_consistent_with_eachother() -> Result<(), subxt::Error> {
    let ctx = test_context().await;
    let api = ctx.client();

    let mut all_sub = api.blocks().subscribe_all().await?;
    let mut best_sub = api.blocks().subscribe_best().await?;
    let mut finalized_sub = api.blocks().subscribe_finalized().await?;

    let mut finals = vec![];
    let mut bests = vec![];
    let mut alls = vec![];

    // Finalization can run behind a bit; blocks that were reported a while ago can
    // only just now be being finalized (in the new RPCs this isn't true and we'll be
    // told about all of those blocks up front). So, first we wait until finalization reports
    // a block that we've seen as new.
    loop {
        tokio::select! {biased;
            Some(Ok(b)) = all_sub.next() => alls.push(b.hash()),
            Some(Ok(b)) = best_sub.next() => bests.push(b.hash()),
            Some(Ok(b)) = finalized_sub.next() => if alls.contains(&b.hash()) { break },
        }
    }

    // Now, gather a couple more finalized blocks as well as anything else we hear about.
    while finals.len() < 2 {
        tokio::select! {biased;
            Some(Ok(b)) = all_sub.next() => alls.push(b.hash()),
            Some(Ok(b)) = best_sub.next() => bests.push(b.hash()),
            Some(Ok(b)) = finalized_sub.next() => finals.push(b.hash()),
        }
    }

    // Check that the items in the first slice are found in the same order in the second slice.
    fn are_same_order_in<T: PartialEq>(a_items: &[T], b_items: &[T]) -> bool {
        let mut b_idx = 0;
        for a in a_items {
            if let Some((idx, _)) = b_items[b_idx..]
                .iter()
                .enumerate()
                .find(|(_idx, b)| a == *b)
            {
                b_idx += idx;
            } else {
                return false;
            }
        }
        true
    }

    // Final blocks and best blocks should both be subsets of _all_ of the blocks reported.
    assert!(
        are_same_order_in(&bests, &alls),
        "Best set {bests:?} should be a subset of all: {alls:?}"
    );
    assert!(
        are_same_order_in(&finals, &alls),
        "Final set {finals:?} should be a subset of all: {alls:?}"
    );

    Ok(())
}

#[tokio::test]
async fn finalized_headers_subscription() -> Result<(), subxt::Error> {
    let ctx = test_context().await;
    let api = ctx.client();

    let mut sub = api.blocks().subscribe_finalized().await?;

    // check that the finalized block reported lines up with the `latest_finalized_block_ref`.
    for _ in 0..2 {
        let header = sub.next().await.unwrap()?;
        let finalized_hash = api.backend().latest_finalized_block_ref().await?.hash();
        assert_eq!(header.hash(), finalized_hash);
    }

    Ok(())
}

#[tokio::test]
async fn missing_block_headers_will_be_filled_in() -> Result<(), subxt::Error> {
    use subxt::backend::legacy;

    let ctx = test_context().await;
    let rpc = ctx.legacy_rpc_methods().await;

    // Manually subscribe to the next 6 finalized block headers, but deliberately
    // filter out some in the middle so we get back b _ _ b _ b. This guarantees
    // that there will be some gaps, even if there aren't any from the subscription.
    let some_finalized_blocks = rpc
        .chain_subscribe_finalized_heads()
        .await?
        .enumerate()
        .take(6)
        .filter(|(n, _)| {
            let n = *n;
            async move { n == 0 || n == 3 || n == 5 }
        })
        .map(|(_, r)| r);

    // This should spot any gaps in the middle and fill them back in.
    let all_finalized_blocks =
        legacy::subscribe_to_block_headers_filling_in_gaps(rpc, some_finalized_blocks, None);
    futures::pin_mut!(all_finalized_blocks);

    // Iterate the block headers, making sure we get them all in order.
    let mut last_block_number = None;
    while let Some(header) = all_finalized_blocks.next().await {
        let header = header?;

        use subxt::config::Header;
        let block_number: u128 = header.number().into();

        if let Some(last) = last_block_number {
            assert_eq!(last + 1, block_number);
        }
        last_block_number = Some(block_number);
    }

    assert!(last_block_number.is_some());
    Ok(())
}

// Check that we can subscribe to non-finalized blocks.
#[tokio::test]
async fn runtime_api_call() -> Result<(), subxt::Error> {
    let ctx = test_context().await;
    let api = ctx.client();
    let rpc = ctx.legacy_rpc_methods().await;

    let mut sub = api.blocks().subscribe_best().await?;

    let block = sub.next().await.unwrap()?;
    let rt = block.runtime_api().await?;

    // get metadata via state_call.
    let (_, meta1) = rt
        .call_raw::<(Compact<u32>, Metadata)>("Metadata_metadata", None)
        .await?;

    // get metadata via `state_getMetadata`.
    let meta2 = rpc.state_get_metadata(Some(block.hash())).await?;

    // They should be the same.
    assert_eq!(meta1.encode(), meta2.encode());

    Ok(())
}

#[tokio::test]
async fn fetch_block_and_decode_extrinsic_details() {
    let ctx = test_context().await;
    let api = ctx.client();

    let alice = dev::alice();
    let bob = dev::bob();

    // Setup; put an extrinsic into a block:
    let tx = node_runtime::tx()
        .balances()
        .transfer_allow_death(bob.public_key().into(), 10_000);

    let signed_extrinsic = api
        .tx()
        .create_signed(&tx, &alice, Default::default())
        .await
        .unwrap();

    let in_block = signed_extrinsic
        .submit_and_watch()
        .await
        .unwrap()
        .wait_for_in_block()
        .await
        .unwrap();

    // Now, separately, download that block. Let's see what it contains..
    let block_hash = in_block.block_hash();
    let block = api.blocks().at(block_hash).await.unwrap();
    let extrinsics = block.extrinsics().await.unwrap();

    assert_eq!(extrinsics.block_hash(), block_hash);

    // `.has` should work and find a transfer call.
    assert!(extrinsics
        .has::<node_runtime::balances::calls::types::TransferAllowDeath>()
        .unwrap());

    // `.find_first` should similarly work to find the transfer call:
    assert!(extrinsics
        .find_first::<node_runtime::balances::calls::types::TransferAllowDeath>()
        .unwrap()
        .is_some());

    let block_extrinsics = extrinsics
        .iter()
        .map(|res| res.unwrap())
        .collect::<Vec<_>>();

    // All blocks contain a timestamp; check this first:
    let timestamp = block_extrinsics.get(0).unwrap();
    timestamp.as_root_extrinsic::<node_runtime::Call>().unwrap();
    timestamp
        .as_extrinsic::<node_runtime::timestamp::calls::types::Set>()
        .unwrap();
    assert!(!timestamp.is_signed());

    // Next we expect our transfer:
    let tx = block_extrinsics.get(1).unwrap();
    tx.as_root_extrinsic::<node_runtime::Call>().unwrap();
    let ext = tx
        .as_extrinsic::<node_runtime::balances::calls::types::TransferAllowDeath>()
        .unwrap()
        .unwrap();
    assert_eq!(ext.value, 10_000);
    assert!(tx.is_signed());
}
