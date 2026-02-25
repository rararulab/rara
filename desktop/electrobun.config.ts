import type { ElectrobunConfig } from "electrobun";

export default {
  app: {
    name: "Rara Desktop",
    identifier: "com.crrow.job.desktop",
    version: "0.0.17",
    description: "Desktop shell for the Rara/Job local dev stack",
  },
  runtime: {
    exitOnLastWindowClosed: true,
  },
  build: {
    bun: {
      entrypoint: "src/bun/index.ts",
      sourcemap: "inline",
      minify: false,
    },
    mac: {
      defaultRenderer: "native",
      bundleCEF: false,
    },
    win: {
      defaultRenderer: "native",
      bundleCEF: false,
    },
    linux: {
      defaultRenderer: "native",
      bundleCEF: false,
    },
  },
} satisfies ElectrobunConfig;
