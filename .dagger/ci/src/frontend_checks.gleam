import ci
import dagger/container
import dagger/mod
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
