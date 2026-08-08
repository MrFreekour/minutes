fn main() {
    #[cfg(target_os = "macos")]
    minutes_core::graph_worker::run_policy_projection_xpc_service_main();

    #[cfg(not(target_os = "macos"))]
    std::process::exit(minutes_core::graph_worker::run_policy_projection_stream_worker_main());
}
