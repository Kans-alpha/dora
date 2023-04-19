use crate::tcp_utils::{tcp_receive, tcp_send};

use dora_core::{
    daemon_messages::{DaemonCoordinatorEvent, DaemonCoordinatorReply, SpawnDataflowNodes},
    descriptor::Descriptor,
};
use eyre::{bail, eyre, ContextCompat, WrapErr};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};
use tokio::net::TcpStream;
use uuid::Uuid;

#[tracing::instrument(skip(daemon_connections))]
pub async fn spawn_dataflow(
    dataflow_path: &Path,
    daemon_connections: &mut HashMap<String, TcpStream>,
) -> eyre::Result<SpawnedDataflow> {
    let descriptor = Descriptor::read(dataflow_path).await.wrap_err_with(|| {
        format!(
            "failed to read dataflow descriptor at {}",
            dataflow_path.display()
        )
    })?;
    descriptor.check(dataflow_path)?;
    let working_dir = dataflow_path
        .canonicalize()
        .context("failed to canoncialize dataflow path")?
        .parent()
        .ok_or_else(|| eyre!("canonicalized dataflow path has no parent"))?
        .to_owned();
    let nodes = descriptor.resolve_aliases_and_set_defaults();
    let uuid = Uuid::new_v4();

    let machines: BTreeSet<_> = nodes.iter().map(|n| n.deploy.machine.clone()).collect();

    let spawn_command = SpawnDataflowNodes {
        dataflow_id: uuid,
        working_dir,
        nodes,
        communication: descriptor.communication,
    };
    let message = serde_json::to_vec(&DaemonCoordinatorEvent::Spawn(spawn_command))?;

    for machine in &machines {
        tracing::trace!("Spawning dataflow `{uuid}` on machine `{machine}`");
        spawn_dataflow_on_machine(daemon_connections, machine, &message)
            .await
            .wrap_err_with(|| format!("failed to spawn dataflow on machine `{machine}`"))?;
    }

    tracing::info!("successfully spawned dataflow `{uuid}`");

    Ok(SpawnedDataflow { uuid, machines })
}

async fn spawn_dataflow_on_machine(
    daemon_connections: &mut HashMap<String, TcpStream>,
    machine: &str,
    message: &[u8],
) -> Result<(), eyre::ErrReport> {
    let daemon_connection = daemon_connections
        .get_mut(machine)
        .wrap_err_with(|| format!("no daemon connection for machine `{machine}`"))?;
    tcp_send(daemon_connection, message)
        .await
        .wrap_err("failed to send spawn message to daemon")?;
    let reply_raw = tcp_receive(daemon_connection)
        .await
        .wrap_err("failed to receive spawn reply from daemon")?;
    match serde_json::from_slice(&reply_raw)
        .wrap_err("failed to deserialize spawn reply from daemon")?
    {
        DaemonCoordinatorReply::SpawnResult(result) => result
            .map_err(|e| eyre!(e))
            .wrap_err("daemon returned an error")?,
        _ => bail!("unexpected reply"),
    }
    Ok(())
}

pub struct SpawnedDataflow {
    pub uuid: Uuid,
    pub machines: BTreeSet<String>,
}
