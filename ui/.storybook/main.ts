import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  framework: { name: "@storybook/react-vite", options: {} },
  staticDirs: ["./public"],
  async viteFinal(config) {
    config.build ??= {};
    config.build.rollupOptions = {
      ...config.build.rollupOptions,
      onwarn(warning, warn) {
        // zod ships `@__PURE__` annotations in positions Rollup will not read.
        // They are advisory, and Storybook otherwise treats the warning as
        // fatal.
        if (warning.code === "INVALID_ANNOTATION") return;
        warn(warning);
      },
    };
    return config;
  },
};

export default config;
