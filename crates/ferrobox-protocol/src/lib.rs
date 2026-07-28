//! Versioned host-to-guest gRPC protocol.

pub mod guest {
    pub mod v1 {
        tonic::include_proto!("ferrobox.guest.v1");
    }
}
