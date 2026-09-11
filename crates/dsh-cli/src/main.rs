//! multitool 入口：neo / neo exec / neo serve / neo resume / neo mcp-server
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("exec")  => println!("{}", dsh_exec::run(&args[1..].join(" "))),
        Some("serve") => println!("[web] listening on 127.0.0.1:3080"),
        Some("resume")=> println!("[resume] restoring from latest checkpoint"),
        _             => println!("[tui] starting interactive session"),
    }
}
