//! Generating the light client's config file.
//!
//! The upstream templates are vendored so the bootnode list stays
//! authoritative; only the paths and the two ports are ours.

use std::path::{Path, PathBuf};

use lantern_sdk_schema::Network;
use toml::Table;
use toml::Value;

use crate::error::BackendError;

const MAINNET_TEMPLATE: &str = include_str!("../assets/mainnet.toml");
const TESTNET_TEMPLATE: &str = include_str!("../assets/testnet.toml");

/// The subdirectory name a network's store lives under.
const fn network_slug(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
    }
}

/// Everything Lantern controls in a light-client config.
#[derive(Debug, Clone)]
pub struct LightClientConfig {
    /// Root for this client's files. Everything actually lands in a
    /// per-network subdirectory of it — see [`Self::network_dir`].
    pub data_dir: PathBuf,
    pub network: Network,
    /// JSON-RPC port, allocated fresh on every spawn.
    pub rpc_port: u16,
    /// P2P port, also dynamic: two profiles sharing 8118 would collide.
    pub p2p_port: u16,
}

impl LightClientConfig {
    const fn template(&self) -> &'static str {
        match self.network {
            Network::Mainnet => MAINNET_TEMPLATE,
            Network::Testnet => TESTNET_TEMPLATE,
        }
    }

    /// Where this network's store, peer database and generated config live.
    ///
    /// A per-network subdirectory rather than an equality check: `data_dir`
    /// is supplied by the caller and nothing else stops the same one being
    /// handed to a mainnet and a testnet client, which would put two chains
    /// into one `RocksDB` store. Separating them structurally makes the
    /// collision unrepresentable instead of merely detectable.
    pub fn network_dir(&self) -> PathBuf {
        self.data_dir.join(network_slug(self.network))
    }

    /// Render the config, keeping the template's bootnodes and replacing only
    /// the paths and ports.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Spawn`] if the vendored template fails to
    /// parse as TOML, is missing an expected table, or fails to serialize.
    pub fn render(&self) -> Result<String, BackendError> {
        let mut doc: Table = self.template().parse().map_err(|e| {
            BackendError::Spawn(format!("vendored template is not valid TOML: {e}"))
        })?;

        let dir = self.network_dir().display().to_string();
        set_path(&mut doc, "store", "path", format!("{dir}/store"))?;
        set_path(&mut doc, "network", "path", format!("{dir}/network"))?;

        let network = doc
            .get_mut("network")
            .and_then(Value::as_table_mut)
            .ok_or_else(|| BackendError::Spawn("template has no [network] table".into()))?;
        network.insert(
            "listen_addresses".to_string(),
            Value::Array(vec![Value::String(format!(
                "/ip4/127.0.0.1/tcp/{}",
                self.p2p_port
            ))]),
        );

        let rpc = doc
            .entry("rpc".to_string())
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| BackendError::Spawn("[rpc] is not a table".into()))?;
        rpc.insert(
            "listen_address".to_string(),
            Value::String(format!("127.0.0.1:{}", self.rpc_port)),
        );

        toml::to_string(&doc)
            .map_err(|e| BackendError::Spawn(format!("could not render config: {e}")))
    }

    /// Render and write, creating the data directory if needed.
    ///
    /// # Errors
    ///
    /// Returns a [`BackendError`] if rendering fails or if any directory or
    /// the config file itself cannot be created.
    pub fn write_to(&self, path: &Path) -> Result<(), BackendError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let dir = self.network_dir();
        std::fs::create_dir_all(dir.join("store"))?;
        std::fs::create_dir_all(dir.join("network"))?;
        std::fs::write(path, self.render()?)?;
        Ok(())
    }
}

fn set_path(doc: &mut Table, table: &str, key: &str, value: String) -> Result<(), BackendError> {
    doc.entry(table.to_string())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| BackendError::Spawn(format!("[{table}] is not a table")))?
        .insert(key.to_string(), Value::String(value));
    Ok(())
}

#[cfg(test)]
mod tests {
    use lantern_sdk_schema::Network;

    use super::LightClientConfig;

    fn config() -> LightClientConfig {
        LightClientConfig {
            data_dir: std::path::PathBuf::from("/tmp/lantern-test"),
            network: Network::Testnet,
            rpc_port: 45_001,
            p2p_port: 45_002,
        }
    }

    #[test]
    fn the_rendered_config_binds_both_ports_to_localhost() {
        let rendered = config().render().expect("renders");
        assert!(
            rendered.contains("127.0.0.1:45001"),
            "rpc port missing:\n{rendered}"
        );
        assert!(
            rendered.contains("/ip4/127.0.0.1/tcp/45002"),
            "p2p port missing:\n{rendered}"
        );
        assert!(
            !rendered.contains("0.0.0.0"),
            "must not listen on every interface:\n{rendered}"
        );
        assert!(
            !rendered.contains("127.0.0.1:9000"),
            "the template's fixed port must be overwritten:\n{rendered}"
        );
    }

    #[test]
    fn paths_live_under_the_supplied_data_dir() {
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("/tmp/lantern-test"), "{rendered}");
    }

    #[test]
    fn the_chain_matches_the_network() {
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("chain = \"testnet\""), "{rendered}");

        let mainnet = LightClientConfig {
            network: Network::Mainnet,
            ..config()
        };
        let rendered = mainnet.render().expect("renders");
        assert!(rendered.contains("chain = \"mainnet\""), "{rendered}");
    }

    #[test]
    fn the_bootnodes_survive_from_the_vendored_template() {
        // The one part we must never invent. If this is empty the client
        // cannot find peers and sync silently never starts.
        let rendered = config().render().expect("renders");
        assert!(rendered.contains("bootnodes"), "{rendered}");
        assert!(
            rendered.contains("/ip4/"),
            "no bootnode entries:\n{rendered}"
        );
    }

    #[test]
    fn two_networks_sharing_a_data_dir_do_not_share_a_store() {
        // Nothing stops a caller handing the same `data_dir` to both, and one
        // RocksDB store holding two chains is not a recoverable state.
        let testnet = config();
        let mainnet = LightClientConfig {
            network: Network::Mainnet,
            ..config()
        };
        assert_ne!(testnet.network_dir(), mainnet.network_dir());
        assert!(
            testnet
                .render()
                .expect("renders")
                .contains("/testnet/store"),
            "{}",
            testnet.render().expect("renders")
        );
        assert!(
            mainnet
                .render()
                .expect("renders")
                .contains("/mainnet/store"),
            "{}",
            mainnet.render().expect("renders")
        );
    }

    #[test]
    fn rendering_is_deterministic() {
        assert_eq!(config().render().expect("a"), config().render().expect("b"));
    }
}
