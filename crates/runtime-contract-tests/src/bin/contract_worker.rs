fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("worker runtime");
    let code = runtime.block_on(runtime_core::worker::run_worker_entrypoint(&args));
    std::process::exit(code);
}
