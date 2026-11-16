use anyhow::Context;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--demo") {
        return qorvi_tui::run_demo().context("fallo en la interfaz de demostración");
    }
    if args.len() == 2 && args[0] == "doctor" && args[1] == "keys" {
        return doctor_keys();
    }
    println!("qorvi {}", env!("CARGO_PKG_VERSION"));
    println!("Usa `qorvi --demo` para ver el mockup de la interfaz.");
    println!("Usa `qorvi doctor keys` para diagnosticar los atajos de tu terminal.");
    Ok(())
}

/// Imprime el informe de atajos; consulta a la terminal con el modo raw activo un instante.
fn doctor_keys() -> anyhow::Result<()> {
    let enhancement = qorvi_tui::probe_keyboard_enhancement_raw()
        .context("no se pudo consultar el teclado de la terminal")?;
    let term_program = std::env::var("TERM_PROGRAM").ok();
    print!(
        "{}",
        qorvi_tui::detect::doctor_keys_report(
            term_program.as_deref(),
            cfg!(target_os = "macos"),
            cfg!(windows),
            enhancement,
        )
    );
    Ok(())
}
