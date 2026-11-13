//! Pure `std::net` TCP 1v1 Lockstep Multiplayer transport for *Wildfront Battlegrounds*.
//!
//! Uses length-prefixed UTF-8 RON packets over background reader/writer threads bridged via
//! `std::sync::mpsc` so the 60 FPS `wgpu` + `egui` render loop never blocks on socket I/O.
//! Host plays as Team 1; Client plays as Team 2; both peers share the exact `MatchInit` seed
//! and `SplitMix64` PRNG so every D6 roll, card draw, and Graveyard tick stays in lockstep.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use hexx::Hex;
use serde::{Deserialize, Serialize};
use wf_core::{DeckList, ScenarioId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LockstepCommand {
    DeployCard {
        team_id: u8,
        hand_idx: usize,
        hex: Hex,
    },
    MoveUnit {
        team_id: u8,
        from_hex: Hex,
        to_hex: Hex,
    },
    AttackUnit {
        team_id: u8,
        attacker_hex: Hex,
        defender_hex: Hex,
    },
    CastAbility {
        team_id: u8,
        caster_hex: Hex,
        target_hex: Hex,
    },
    RestUnit {
        team_id: u8,
        unit_hex: Hex,
    },
    EndTurn {
        team_id: u8,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetPacket {
    MatchInit {
        scenario_id: ScenarioId,
        seed: u64,
        host_deck: DeckList,
        fog_enabled: bool,
    },
    ClientJoinDeck {
        client_deck: DeckList,
    },
    Command(LockstepCommand),
    DisconnectNotice,
}

#[derive(Debug)]
pub enum NetEvent {
    PeerConnected(String),
    PacketReceived(NetPacket),
    ErrorOrClosed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetStatus {
    Offline,
    HostingWait(String),
    ConnectedHost(String),
    ConnectedClient(String),
}

pub struct NetLockstepManager {
    pub status: NetStatus,
    pub host_bind_input: String,
    pub join_addr_input: String,
    pub status_line: String,
    rx_events: Option<Receiver<NetEvent>>,
    tx_packets: Option<Sender<NetPacket>>,
    cancel_flag: Arc<AtomicBool>,
}

impl Default for NetLockstepManager {
    fn default() -> Self {
        Self {
            status: NetStatus::Offline,
            host_bind_input: "0.0.0.0:20720".to_string(),
            join_addr_input: "127.0.0.1:20720".to_string(),
            status_line: "Offline (Solo / Hotseat)".to_string(),
            rx_events: None,
            tx_packets: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn spawn_stream_workers(
    stream: TcpStream,
    peer_label: String,
    ev_tx: Sender<NetEvent>,
) -> Sender<NetPacket> {
    let _ = stream.set_nodelay(true);
    let read_stream = stream.try_clone().expect("Failed cloning TCP stream");
    let mut write_stream = stream;

    let (pkt_tx, pkt_rx) = channel::<NetPacket>();

    // Outbound writer thread
    let ev_tx_w = ev_tx.clone();
    thread::spawn(move || {
        while let Ok(pkt) = pkt_rx.recv() {
            let Ok(encoded) = ron::to_string(&pkt) else {
                continue;
            };
            let bytes = encoded.as_bytes();
            let len_be = (bytes.len() as u32).to_be_bytes();
            if write_stream.write_all(&len_be).is_err()
                || write_stream.write_all(bytes).is_err()
                || write_stream.flush().is_err()
            {
                let _ = ev_tx_w.send(NetEvent::ErrorOrClosed("TCP send disconnected.".into()));
                break;
            }
        }
    });

    // Inbound reader thread
    thread::spawn(move || {
        let _ = ev_tx.send(NetEvent::PeerConnected(peer_label));
        let mut r_sock = read_stream;
        loop {
            let mut len_buf = [0u8; 4];
            if let Err(e) = r_sock.read_exact(&mut len_buf) {
                let _ = ev_tx.send(NetEvent::ErrorOrClosed(format!("Peer closed connection ({e})")));
                break;
            }
            let payload_len = u32::from_be_bytes(len_buf) as usize;
            if payload_len == 0 || payload_len > 4 * 1024 * 1024 {
                let _ = ev_tx.send(NetEvent::ErrorOrClosed("Invalid packet frame size.".into()));
                break;
            }
            let mut buf = vec![0u8; payload_len];
            if let Err(e) = r_sock.read_exact(&mut buf) {
                let _ = ev_tx.send(NetEvent::ErrorOrClosed(format!("Read interrupted ({e})")));
                break;
            }
            match std::str::from_utf8(&buf)
                .ok()
                .and_then(|s| ron::from_str::<NetPacket>(s).ok())
            {
                Some(pkt) => {
                    let is_disc = matches!(pkt, NetPacket::DisconnectNotice);
                    let _ = ev_tx.send(NetEvent::PacketReceived(pkt));
                    if is_disc {
                        break;
                    }
                }
                None => {
                    let _ = ev_tx.send(NetEvent::ErrorOrClosed("Malformed RON packet.".into()));
                    break;
                }
            }
        }
    });

    pkt_tx
}

impl NetLockstepManager {
    pub fn is_online(&self) -> bool {
        matches!(
            self.status,
            NetStatus::ConnectedHost(_) | NetStatus::ConnectedClient(_)
        )
    }

    pub fn disconnect(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        if let Some(tx) = self.tx_packets.take() {
            let _ = tx.send(NetPacket::DisconnectNotice);
        }
        self.rx_events = None;
        self.status = NetStatus::Offline;
        self.status_line = "Offline (Solo / Hotseat)".to_string();
    }

    pub fn start_hosting(&mut self) -> Result<(), String> {
        self.disconnect();
        let bind_addr = self.host_bind_input.trim().to_string();
        let listener = TcpListener::bind(&bind_addr)
            .map_err(|e| format!("Could not bind {}: {}", bind_addr, e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Failed setting nonblocking listener: {e}"))?;

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = cancel.clone();
        let (ev_tx, ev_rx) = channel::<NetEvent>();
        let (bridge_tx, bridge_rx) = channel::<NetPacket>();

        let bind_clone = bind_addr.clone();
        thread::spawn(move || {
            while !cancel.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer_addr)) => {
                        let _ = stream.set_nonblocking(false);
                        let worker_tx =
                            spawn_stream_workers(stream, peer_addr.to_string(), ev_tx.clone());
                        while let Ok(pkt) = bridge_rx.recv() {
                            if worker_tx.send(pkt).is_err() {
                                break;
                            }
                        }
                        break;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(30));
                    }
                    Err(e) => {
                        let _ = ev_tx.send(NetEvent::ErrorOrClosed(format!(
                            "Accept error on {}: {}",
                            bind_clone, e
                        )));
                        break;
                    }
                }
            }
        });

        self.rx_events = Some(ev_rx);
        self.tx_packets = Some(bridge_tx);
        self.status = NetStatus::HostingWait(bind_addr.clone());
        self.status_line = format!("⏳ Hosting on {} — waiting for Player 2...", bind_addr);
        Ok(())
    }

    pub fn connect_to_host(&mut self) -> Result<(), String> {
        self.disconnect();
        let addr = self.join_addr_input.trim().to_string();
        let stream = TcpStream::connect(&addr)
            .map_err(|e| format!("Failed connecting to {}: {}", addr, e))?;
        let (ev_tx, ev_rx) = channel::<NetEvent>();
        let pkt_tx = spawn_stream_workers(stream, addr.clone(), ev_tx);

        self.rx_events = Some(ev_rx);
        self.tx_packets = Some(pkt_tx);
        self.status = NetStatus::ConnectedClient(addr.clone());
        self.status_line = format!("🌐 Connected to Host {} (Team 2)", addr);
        Ok(())
    }

    pub fn send(&self, pkt: NetPacket) {
        if let Some(tx) = &self.tx_packets {
            let _ = tx.send(pkt);
        }
    }

    pub fn poll_events(&mut self) -> Vec<NetEvent> {
        let mut out = Vec::new();
        let mut closed = false;
        if let Some(rx) = &self.rx_events {
            while let Ok(ev) = rx.try_recv() {
                match &ev {
                    NetEvent::PeerConnected(peer) => {
                        if matches!(self.status, NetStatus::HostingWait(_)) {
                            self.status = NetStatus::ConnectedHost(peer.clone());
                            self.status_line = format!("🌐 Hosting vs {} (Team 1)", peer);
                        }
                    }
                    NetEvent::ErrorOrClosed(msg) => {
                        self.status_line = format!("⚠ {msg}");
                        closed = true;
                    }
                    NetEvent::PacketReceived(NetPacket::DisconnectNotice) => {
                        self.status_line = "⚠ Remote peer disconnected.".to_string();
                        closed = true;
                    }
                    _ => {}
                }
                out.push(ev);
            }
        }
        if closed {
            self.status = NetStatus::Offline;
            self.rx_events = None;
            self.tx_packets = None;
        }
        out
    }

    /// Binds on an OS-assigned ephemeral loopback port (`127.0.0.1:0`) for conflict-free testing.
    pub fn start_hosting_ephemeral(&mut self) -> Result<u16, String> {
        self.disconnect();
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed binding ephemeral port: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("local_addr error: {e}"))?
            .port();
        self.host_bind_input = format!("127.0.0.1:{port}");
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking error: {e}"))?;

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = cancel.clone();
        let (ev_tx, ev_rx) = channel::<NetEvent>();
        let (bridge_tx, bridge_rx) = channel::<NetPacket>();

        thread::spawn(move || {
            while !cancel.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer_addr)) => {
                        let _ = stream.set_nonblocking(false);
                        let worker_tx =
                            spawn_stream_workers(stream, peer_addr.to_string(), ev_tx.clone());
                        while let Ok(pkt) = bridge_rx.recv() {
                            if worker_tx.send(pkt).is_err() {
                                break;
                            }
                        }
                        break;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15));
                    }
                    Err(_) => break,
                }
            }
        });

        self.rx_events = Some(ev_rx);
        self.tx_packets = Some(bridge_tx);
        self.status = NetStatus::HostingWait(self.host_bind_input.clone());
        Ok(port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wf_core::OrganismFaction;

    #[test]
    fn tcp_loopback_handshake_and_lockstep_packet_roundtrip() {
        let mut host = NetLockstepManager::default();
        let port = host.start_hosting_ephemeral().expect("Ephemeral bind failed");

        let mut client = NetLockstepManager::default();
        client.join_addr_input = format!("127.0.0.1:{port}");
        client.connect_to_host().expect("Client connect failed");

        let dummy_deck = DeckList {
            deck_name: "Loopback Test Deck".into(),
            faction: wf_core::OriginAllegiance::Australia,
            primary_faction: OrganismFaction::Plant,
            secondary_faction: OrganismFaction::Cnidarian,
            is_pure_biotype: false,
            point_budget: 250,
            entries: vec![],
        };
        host.send(NetPacket::MatchInit {
            scenario_id: ScenarioId::EmergenceSkirmish,
            seed: 0x2072_CAFE,
            host_deck: dummy_deck,
            fog_enabled: true,
        });
        client.send(NetPacket::Command(LockstepCommand::EndTurn { team_id: 2 }));

        let mut client_got_init = false;
        let mut host_got_cmd = false;
        for _ in 0..50 {
            for ev in client.poll_events() {
                if let NetEvent::PacketReceived(NetPacket::MatchInit { seed, .. }) = ev {
                    assert_eq!(seed, 0x2072_CAFE);
                    client_got_init = true;
                }
            }
            for ev in host.poll_events() {
                if let NetEvent::PacketReceived(NetPacket::Command(LockstepCommand::EndTurn { team_id })) = ev {
                    assert_eq!(team_id, 2);
                    host_got_cmd = true;
                }
            }
            if client_got_init && host_got_cmd {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        host.disconnect();
        client.disconnect();
        assert!(client_got_init, "Client did not receive MatchInit over TCP");
        assert!(host_got_cmd, "Host did not receive LockstepCommand over TCP");
    }
}
