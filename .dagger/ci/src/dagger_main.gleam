import ci
import dagger/client
import dagger/container
import dagger/directory
import dagger/mod
import dagger/mod/argument
import dagger/mod/codec
import dagger/types

pub fn frontend(
  context: mod.Context,
  source: types.Directory,
) -> Result(String, codec.Error) {
  ci.frontend(context.client, source, "ui")
  |> container.with_exec(["bun", "run", "storybook:build"])
  |> ci.finish
}

pub fn rust(
  context: mod.Context,
  source: types.Directory,
) -> Result(String, codec.Error) {
  let ui =
    ci.frontend(context.client, source, "ui")
    |> container.directory("/src/ui/dist")
  ci.image(context.client, "rust:1.99-bookworm")
  |> container.with_mounted_cache(
    "/usr/local/cargo/registry",
    client.cache_volume(context.client, "gateway-cargo-registry"),
  )
  |> container.with_mounted_cache(
    "/src/target",
    client.cache_volume(context.client, "gateway-rust-1.99-target"),
  )
  |> container.with_directory("/src", ci.clean(source))
  |> container.with_directory("/src/ui/dist", ui)
  |> container.with_workdir("/src")
  |> ci.commands([
    ["cargo", "fmt", "--all", "--", "--check"],
    [
      "cargo",
      "clippy",
      "--locked",
      "--all-targets",
      "--all-features",
      "--",
      "-D",
      "warnings",
    ],
    ["cargo", "test", "--locked", "--all-features"],
  ])
  |> ci.finish
}

pub fn definition() -> mod.Object {
  mod.object("ProjectCi")
  |> mod.with_function(mod.function(
    "frontend",
    argument.required("source", directory.codec()),
    codec.string(),
    frontend,
  ))
  |> mod.with_function(mod.function(
    "rust",
    argument.required("source", directory.codec()),
    codec.string(),
    rust,
  ))
}

pub fn main() {
  ci.serve(definition())
}
