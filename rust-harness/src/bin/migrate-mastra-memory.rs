use std::{env, path::PathBuf};

use agent_arena_npc_harness::{
    memory::migration::{MastraMigrationOptions, migrate_mastra_memory},
    observability,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    observability::init_tracing();
    let arguments = Arguments::parse()?;
    let report = migrate_mastra_memory(
        &MastraMigrationOptions {
            character_id: &arguments.character,
            source_database: &arguments.source,
            destination_database: &arguments.destination,
            legacy_conversations_file: arguments.legacy_conversations.as_deref(),
            visited_file: arguments.visited.as_deref(),
        },
        observability::tracing_sink(),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct Arguments {
    character: String,
    source: PathBuf,
    destination: PathBuf,
    legacy_conversations: Option<PathBuf>,
    visited: Option<PathBuf>,
}

impl Arguments {
    fn parse() -> anyhow::Result<Self> {
        let mut character = None;
        let mut source = None;
        let mut destination = None;
        let mut legacy_conversations = None;
        let mut visited = None;
        let mut values = env::args().skip(1);
        while let Some(flag) = values.next() {
            let value = values
                .next()
                .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
            match flag.as_str() {
                "--character" => character = Some(value),
                "--source" => source = Some(value.into()),
                "--destination" => destination = Some(value.into()),
                "--legacy-conversations" => legacy_conversations = Some(value.into()),
                "--visited" => visited = Some(value.into()),
                _ => anyhow::bail!("unknown argument {flag}"),
            }
        }
        Ok(Self {
            character: character.ok_or_else(|| anyhow::anyhow!("--character is required"))?,
            source: source.ok_or_else(|| anyhow::anyhow!("--source is required"))?,
            destination: destination.ok_or_else(|| anyhow::anyhow!("--destination is required"))?,
            legacy_conversations,
            visited,
        })
    }
}
