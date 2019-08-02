//#[macro_use]
extern crate serde_derive;

use bincode::{deserialize, serialize};
use libcommon_rs::peer::{Peer, PeerId, PeerList};
use libtransport::errors::{Error, Error::AtMaxVecCapacity, Result};
use libtransport::Transport;
use serde::Serialize;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::thread::JoinHandle;

//#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TCPtransport<Data> {
    channel_pool: Vec<Sender<Data>>,
    quit_rx: Receiver<()>,
    quit_tx: Sender<()>,
    server_handle: JoinHandle<()>,
}

pub struct TCPtransportCfg {
    bind_net_addr: String,
}

impl<Id, Pe, Data, E, PL> Transport<Id, Data, E, PL> for TCPtransport<Data>
where
    Data: AsRef<u8> + Serialize,
    Id: PeerId,
    Pe: Peer<Id>,
    PL: PeerList<Id, E, Item = Pe>,
{
    type Configuration = TCPtransportCfg;

    fn new(cfg: Self::Configuration) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            // FIXME: what we do with unwrap() in threads?
            let listener = TcpListener::bind(cfg.bind_net_addr).unwrap();
            for stream in listener.incoming() {
                match stream {
                    Err(e) => panic!("error in accepting connection: {}", e),
                    Ok(stream) => {
                        // receive Data and push it into channels, pipes and call callbacks
                    }
                }
            }
        });
        TCPtransport {
            channel_pool: Vec::with_capacity(1),
            quit_rx: rx,
            quit_tx: tx,
            server_handle: handle,
        }
    }

    fn send(&mut self, peer_address: String, data: Data) -> Result<()> {
        let mut stream = TcpStream::connect(peer_address)?;
        let bytes = serialize(&data)?;
        let sent = stream.write(&bytes)?;
        if sent != bytes.len() {
            return Err(Error::Incomplete);
        }
        Ok(())
    }

    fn broadcast(&mut self, peers: &mut PL, data: Data) -> Result<()> {
        for p in peers {
            let mut stream = TcpStream::connect(p.get_net_addr())?;
            let bytes = serialize(&data)?;
            let sent = stream.write(&bytes)?;
            if sent != bytes.len() {
                return Err(Error::Incomplete);
            }
        }
        Ok(())
    }

    fn register_channel(&mut self, sender: Sender<Data>) -> Result<()> {
        // Vec::push() panics when number of elements overflows `usize`
        if self.channel_pool.len() == std::usize::MAX {
            return Err(AtMaxVecCapacity);
        }
        self.channel_pool.push(sender);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
