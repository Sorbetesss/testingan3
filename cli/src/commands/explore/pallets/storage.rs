use clap::Args;
use color_eyre::eyre::eyre;
use indoc::{formatdoc, writedoc};
use scale_typegen_description::type_description;
use std::fmt::Write;
use std::write;

use subxt::OnlineClient;
use subxt::{
    config::SubstrateConfig,
    metadata::{
        types::{PalletMetadata, StorageEntryType, StorageMetadata},
        Metadata,
    },
};

use crate::utils::{first_paragraph_of_docs, type_example, Indent};

#[derive(Debug, Clone, Args)]
pub struct StorageSubcommand {
    storage_entry: Option<String>,
    #[clap(required = false)]
    trailing_args: Vec<String>,
}

pub async fn explore_storage(
    command: StorageSubcommand,
    pallet_metadata: PalletMetadata<'_>,
    metadata: &Metadata,
    custom_url: Option<String>,
    output: &mut impl std::io::Write,
) -> color_eyre::Result<()> {
    let pallet_name = pallet_metadata.name();
    let trailing_args = command.trailing_args.join(" ");
    let trailing_args = trailing_args.trim();

    let Some(storage_metadata) = pallet_metadata.storage() else {
        writeln!(
            output,
            "The \"{pallet_name}\" pallet has no storage entries."
        )?;
        return Ok(());
    };

    let usage = || {
        let storage_entries = storage_entries_string(storage_metadata, pallet_name);
        formatdoc! {"
        Usage:
            subxt explore pallet {pallet_name} storage <STORAGE_ENTRY>
                view details for a specific storage entry
        
        {storage_entries}
        "}
    };

    // if no storage entry specified, show user the calls to choose from:
    let Some(entry_name) = command.storage_entry else {
        writeln!(output, "{}", usage())?;
        return Ok(());
    };

    // if specified call storage entry wrong, show user the storage entries to choose from (but this time as an error):
    let Some(storage) = storage_metadata
        .entries()
        .iter()
        .find(|entry| entry.name().to_lowercase() == entry_name.to_lowercase())
    else {
        return Err(eyre!(
            "Storage entry \"{entry_name}\" not found in \"{pallet_name}\" pallet!\n\n{}",
            usage()
        ));
    };

    let (return_ty_id, key_ty_id) = match storage.entry_type() {
        StorageEntryType::Plain(value) => (*value, None),
        StorageEntryType::Map {
            value_ty, key_ty, ..
        } => (*value_ty, Some(*key_ty)),
    };

    // only inform user about usage if a key can be provided:
    if key_ty_id.is_some() && trailing_args.is_empty() {
        writedoc! {output, "
        Usage:
            subxt explore pallet {pallet_name} storage {entry_name} <KEY_VALUE>
                retrieve a value from storage
        "}?;
    }

    let docs_string = first_paragraph_of_docs(storage.docs()).indent(4);
    if !docs_string.is_empty() {
        writedoc! {output, "

        Storage Entry Docs:
        {docs_string}
        "}?;
    }

    // inform user about shape of key if it can be provided:
    if let Some(key_ty_id) = key_ty_id {
        let key_ty_description = type_description(key_ty_id, metadata.types(), true)
            .expect("No type Description")
            .indent(4);

        let key_ty_example = type_example(key_ty_id, metadata.types()).indent(4);

        writedoc! {output, "

        The <KEY_VALUE> has the following shape:
        {key_ty_description}

        For example you could provide this <KEY_VALUE>:
        {key_ty_example}
        "}?;
    } else {
        writeln!(
            output,
            "The storage entry can be accessed without providing a key."
        )?;
    }

    let return_ty_description = type_description(return_ty_id, metadata.types(), true)
        .expect("No type Description")
        .indent(4);

    writedoc! {output, "
    
    The storage entry has the following shape:
    {return_ty_description}
    "}?;

    // construct the vector of scale_values that should be used as a key to the storage (often empty)

    let key_scale_values = if let Some(key_ty_id) = key_ty_id.filter(|_| !trailing_args.is_empty())
    {
        let key_scale_value = scale_value::stringify::from_str(trailing_args).0.map_err(|err| eyre!("scale_value::stringify::from_str led to a ParseError.\n\ntried parsing: \"{}\"\n\n{}", trailing_args, err))?;
        let key_scale_value_str = key_scale_value.indent(4);
        writedoc! {output, "

        You submitted the following value as a key:
        {key_scale_value_str}
        "}?;

        let mut key_bytes: Vec<u8> = Vec::new();
        scale_value::scale::encode_as_type(
            &key_scale_value,
            key_ty_id,
            metadata.types(),
            &mut key_bytes,
        )?;
        let bytes_composite = scale_value::Value::from_bytes(&key_bytes);
        vec![bytes_composite]
    } else {
        Vec::new()
    };

    if key_ty_id.is_none() && !trailing_args.is_empty() {
        writedoc! {output, "

        Warning: You submitted the following value as a key, but it will be ignored, 
        because the storage entry does not require a key: \"{trailing_args}\"
        "}?;
    }

    // construct and submit the storage entry request if either no key is needed or som key was provided as a scale value
    if key_ty_id.is_none() || !key_scale_values.is_empty() {
        let online_client = match custom_url {
            None => OnlineClient::<SubstrateConfig>::new().await?,
            Some(url) => OnlineClient::<SubstrateConfig>::from_url(url).await?,
        };
        let storage_query = subxt::dynamic::storage(pallet_name, entry_name, key_scale_values);
        let decoded_value_thunk_or_none = online_client
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        let decoded_value_thunk =
            decoded_value_thunk_or_none.ok_or(eyre!("Value not found in storage."))?;

        let value = decoded_value_thunk.to_value()?.indent(4);
        writedoc! {output, "
        
        The value of the storage entry is:
        {value}
        "}?;
    }

    Ok(())
}

fn storage_entries_string(storage_metadata: &StorageMetadata, pallet_name: &str) -> String {
    if storage_metadata.entries().is_empty() {
        format!("No <STORAGE_ENTRY>'s available in the \"{pallet_name}\" pallet.")
    } else {
        let mut output = format!(
            "Available <STORAGE_ENTRY>'s in the \"{}\" pallet:",
            pallet_name
        );
        let mut strings: Vec<_> = storage_metadata
            .entries()
            .iter()
            .map(|s| s.name())
            .collect();
        strings.sort();
        for entry in strings {
            write!(output, "\n    {}", entry).unwrap();
        }
        output
    }
}
