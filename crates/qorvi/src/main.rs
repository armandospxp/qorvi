use anyhow::Context;

fn main() -> anyhow::Result<()> {
    if std::env::args().skip(1).any(|a| a == "--demo") {
        return qorvi_tui::run_demo().context("fallo en la interfaz de demostración");
    }
    println!("qorvi {}", env!("CARGO_PKG_VERSION"));
    println!("Usa `qorvi --demo` para ver el mockup de la interfaz.");
    Ok(())
}
