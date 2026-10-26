import dagger/directory
import dagger/mod
import dagger/mod/argument
import dagger/mod/codec
import frontend_checks
import rust_checks

pub fn definition() -> mod.Object {
  mod.object("ProjectCi")
  |> mod.with_function(mod.function(
    "frontend",
    argument.required("source", directory.codec()),
    codec.string(),
    frontend_checks.frontend,
  ))
  |> mod.with_function(mod.function(
    "rust",
    argument.required("source", directory.codec()),
    codec.string(),
    rust_checks.rust,
  ))
}
