pub mod database {
    include!(concat!(env!("OUT_DIR"), "/database.rs"));
}

pub mod keyserver {
    include!(concat!(env!("OUT_DIR"), "/keyserver.rs"));
}

pub use cashweb::auth_wrapper as wrapper;
