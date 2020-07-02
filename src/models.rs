pub mod database {
    include!(concat!(env!("OUT_DIR"), "/database.rs"));
}

pub use cashweb::keyserver;

pub use cashweb::auth_wrapper as wrapper;
