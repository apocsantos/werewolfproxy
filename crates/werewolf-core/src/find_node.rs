use crate::lookup::lookup;
use crate::message::Message;
use crate::nodeid::NodeId;
use crate::routing::RoutingTable;

pub fn handle_find_node(
    local_id: &NodeId,
    table: &RoutingTable,
    message: &Message,
) -> Option<Message> {
    match message {
        Message::FindNode { target, k, .. } => {
            let result = lookup(table, target, *k);

            Some(Message::FindNodeResult {
                sender: local_id.clone(),
                target: target.clone(),
                peers: result.peers,
            })
        }
        _ => None,
    }
}
