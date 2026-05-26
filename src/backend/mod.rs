macro_rules! variant_map {
    ( fn $fn_name:ident ( $enum_type:ident ) -> $ret:ty {
        $( $variant:ident => $value:expr ),* $(,)?
    }) => {
        fn $fn_name(v: $enum_type) -> $ret {
            match v {
                $( $enum_type::$variant => $value, )*
            }
        }
    };
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Emitter;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Emitter;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod stub;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use stub::Emitter;
