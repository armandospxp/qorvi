//! Árbol de procesos del agente: al cerrarlo se terminan también sus hijos.
//!
//! Muchos agentes se lanzan con un envoltorio (`npx` → `node`, un script → el binario).
//! Matar solo el proceso directo dejaría vivos a los nietos, que siguen gastando tokens.
//! - Unix: el agente es líder de su propio grupo de procesos y se señala el grupo entero.
//! - Windows: el agente entra en un Job Object con «matar al cerrar»; al soltar el job
//!   (o si qorvi muere) Windows termina todos sus procesos.

use tokio::process::{Child, Command};

/// Espera entre la señal amable y la definitiva (Unix).
#[cfg(unix)]
const GRACE: std::time::Duration = std::time::Duration::from_millis(300);

/// Prepara `cmd` para poder terminar su árbol después.
pub(crate) fn prepare(cmd: &mut Command) {
    #[cfg(unix)]
    {
        // 0 = grupo nuevo con el pid del hijo como id. Además, el Ctrl+C de la terminal
        // (que va al grupo en primer plano) ya no le llega.
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = cmd;
}

/// Árbol de procesos de un agente lanzado.
pub(crate) struct ProcessTree {
    #[cfg(unix)]
    pgid: Option<i32>,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl ProcessTree {
    /// Toma el control del árbol de `child` (recién lanzado con [`prepare`]).
    pub(crate) fn attach(child: &Child) -> ProcessTree {
        #[cfg(unix)]
        {
            ProcessTree {
                pgid: child.id().and_then(|p| i32::try_from(p).ok()),
            }
        }
        #[cfg(windows)]
        {
            ProcessTree {
                job: child.raw_handle().and_then(windows::Job::assign),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            ProcessTree {}
        }
    }

    /// Termina los procesos que queden del árbol (el agente y sus descendientes).
    pub(crate) async fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid.take() {
            // SIGTERM para que limpien; SIGKILL a los que sigan. Si el grupo ya no
            // existe, `kill` devuelve ESRCH y no pasa nada.
            // SAFETY: `kill` no toca memoria; un pgid negativo señala al grupo.
            unsafe { libc::kill(-pgid, libc::SIGTERM) };
            tokio::time::sleep(GRACE).await;
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }
        #[cfg(windows)]
        {
            // Cerrar el job termina todos sus procesos.
            self.job.take();
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::RawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Job Object propio; al cerrarlo mueren sus procesos.
    pub(super) struct Job(HANDLE);

    // SAFETY: un HANDLE de job se puede usar y cerrar desde cualquier hilo.
    unsafe impl Send for Job {}

    impl Job {
        /// Crea un job con «matar al cerrar» y mete en él al proceso `process`.
        pub(super) fn assign(process: RawHandle) -> Option<Job> {
            // SAFETY: llamadas Win32 con argumentos válidos; el job se cierra en Drop.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let job = Job(job);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 || AssignProcessToJobObject(job.0, process as HANDLE) == 0 {
                    return None;
                }
                Some(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: el handle es nuestro y se cierra una sola vez.
            unsafe { CloseHandle(self.0) };
        }
    }
}
