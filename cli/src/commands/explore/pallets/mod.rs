use clap::{command, Parser, Subcommand};

use subxt::Metadata;
use subxt_metadata::PalletMetadata;

use crate::utils::{first_paragraph_of_docs, FileOrUrl, Indent};

use self::{calls::CallsSubcommand, constants::ConstantsSubcommand, storage::StorageSubcommand};

use calls::explore_calls;
use constants::explore_constants;
use storage::explore_storage;

mod calls;
mod constants;
mod events;
mod storage;

#[derive(Debug, Clone, Subcommand)]
pub enum PalletSubcommand {
    Calls(CallsSubcommand),
    Constants(ConstantsSubcommand),
    Storage(StorageSubcommand),
}

pub async fn run<'a>(
    subcommand: Option<PalletSubcommand>,
    pallet_metadata: PalletMetadata<'a>,
    metadata: &'a Metadata,
    file_or_url: FileOrUrl,
    output: &mut impl std::io::Write,
) -> color_eyre::Result<()> {
    let pallet_name = pallet_metadata.name();
    let Some(subcommand) = subcommand else {
        let docs_string = first_paragraph_of_docs(pallet_metadata.docs()).indent(4);
        if !docs_string.is_empty() {
            writeln!(output, "Description:\n{docs_string}")?;
        }
        writeln!(output, "Usage:")?;
        writeln!(output, "    subxt explore pallet {pallet_name} calls")?;
        writeln!(
            output,
            "        explore the calls that can be made into this pallet"
        )?;
        writeln!(output, "    subxt explore pallet {pallet_name} constants")?;
        writeln!(output, "        explore the constants held in this pallet")?;
        writeln!(output, "    subxt explore pallet {pallet_name} storage")?;
        writeln!(
            output,
            "        explore the storage values held in this pallet"
        )?;
        return Ok(());
    };

    match subcommand {
        PalletSubcommand::Calls(command) => {
            explore_calls(command, pallet_metadata, metadata, output)
        }
        PalletSubcommand::Constants(command) => {
            explore_constants(command, pallet_metadata, metadata, output)
        }
        PalletSubcommand::Storage(command) => {
            // if the metadata came from some url, we use that same url to make storage calls against.
            let custom_url = file_or_url.url.map(|url| url.to_string());
            explore_storage(command, pallet_metadata, metadata, custom_url, output).await
        }
    }
}
