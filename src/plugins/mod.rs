pub mod c;
pub mod go;
pub mod python;
pub mod rust;
pub mod typescript;
pub mod zig;

use crate::config::PocConfig;
use crate::types::Plugin;

pub fn all_plugins(config: &PocConfig) -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(rust::RustPlugin::new(config)),
        Box::new(go::GoPlugin),
        Box::new(typescript::TypeScriptPlugin::new(config)),
        Box::new(c::CPlugin::new(config)),
        Box::new(python::PythonPlugin::new(config)),
        Box::new(zig::ZigPlugin),
    ]
}
