//! The names that identify this copy of L-Helper to Windows. The test build (cargo feature "test-identity",
//! `packaging\release.ps1 -Test`) has its own set, so it installs and runs next to a regular L-Helper (or a fork
//! using the same names) without touching its files, settings, Dynamic Lighting package, startup task or process.

#[cfg(not(feature = "test-identity"))]
mod names {
    /// Title bar, Start menu, installer and Apps list
    pub const DISPLAY: &str = "L-Helper";
    /// Folder under %ProgramData% (settings, log) and %ProgramFiles% (the installed exe)
    pub const FOLDER: &str = "LHelper";
    /// The installed exe's file name; other copies with this name are the ones a second start may end
    pub const EXE: &str = "lhelper.exe";
    /// Taskbar ID of the window and the Start menu shortcut
    pub const AUMID: &str = "LHelper.LHelper";
    /// Single-instance mutex and its -Show / -Ack events (Global\...)
    pub const INSTANCE: &str = "LHelperRsApp";
    /// Sparse package name, also its certificate's subject (CN=...)
    pub const PACKAGE: &str = "LHelper";
    /// Startup task name prefix (followed by the user name)
    pub const TASK: &str = "LHelper_";
}

#[cfg(feature = "test-identity")]
mod names {
    pub const DISPLAY: &str = "L-Helper Test";
    pub const FOLDER: &str = "LHelperTest";
    pub const EXE: &str = "lhelper-test.exe";
    pub const AUMID: &str = "LHelper.Test";
    pub const INSTANCE: &str = "LHelperTestApp";
    pub const PACKAGE: &str = "LHelperTest";
    pub const TASK: &str = "LHelperTest_";
}

pub use names::*;

pub fn publisher() -> String {
    format!("CN={PACKAGE}")
}

/// All users' Start menu shortcut
pub fn shortcut_name() -> String {
    format!("{DISPLAY}.lnk")
}
