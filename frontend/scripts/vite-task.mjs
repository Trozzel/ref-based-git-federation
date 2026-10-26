import { spawnSync } from "node:child_process";

const [task, ...rawArgs] = process.argv.slice(2);
const viteArgs = [];

function takeValue(args, index, name) {
  const current = args[index];
  if (current.includes("=")) {
    return [current.slice(current.indexOf("=") + 1), index];
  }

  const next = args[index + 1];
  if (!next) {
    throw new Error(`${name} requires a value`);
  }

  return [next, index + 1];
}

for (let index = 0; index < rawArgs.length; index += 1) {
  const arg = rawArgs[index];

  if (arg === "--api-port" || arg.startsWith("--api-port=")) {
    const [value, nextIndex] = takeValue(rawArgs, index, "--api-port");
    process.env.VITE_API_PORT = value;
    index = nextIndex;
    continue;
  }

  if (arg === "--api-base-url" || arg.startsWith("--api-base-url=")) {
    const [value, nextIndex] = takeValue(rawArgs, index, "--api-base-url");
    process.env.VITE_API_BASE_URL = value;
    index = nextIndex;
    continue;
  }

  viteArgs.push(arg);
}

function run(command, args) {
  const result = spawnSync(command, args, {
    env: process.env,
    stdio: "inherit",
    shell: process.platform === "win32"
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

if (task === "build") {
  run("tsc", ["-b"]);
  run("vite", ["build", ...viteArgs]);
} else if (task === "dev") {
  run("vite", ["--host", "127.0.0.1", ...viteArgs]);
} else {
  throw new Error("Expected task to be either dev or build");
}
