const defaultFull = {
  mode: "full",
  items: 100000,
  iterations: 1000,
  runs: 20,
  warmup: 3,
  saveBaseline: false,
  compare: false,
  failOnRegression: false,
  thresholdPercent: 10,
  cases: []
};

const quickOverrides = {
  mode: "quick",
  iterations: 100,
  runs: 5,
  warmup: 1
};

export function parseArgs(argv) {
  const config = { ...defaultFull };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    switch (arg) {
      case "--quick":
        Object.assign(config, quickOverrides);
        break;
      case "--full":
        Object.assign(config, { mode: "full", runs: 20, warmup: 3 });
        break;
      case "--save-baseline":
        config.saveBaseline = true;
        break;
      case "--compare":
        config.compare = true;
        break;
      case "--fail-on-regression":
        config.failOnRegression = true;
        break;
      case "--threshold":
        config.thresholdPercent = requirePositiveNumber(argv, index, "--threshold");
        index += 1;
        break;
      case "--case":
        config.cases.push(requireNonEmptyValue(argv, index, "--case"));
        index += 1;
        break;
      case "--items":
        config.items = requirePositiveInteger(argv, index, "--items");
        index += 1;
        break;
      case "--iterations":
        config.iterations = requirePositiveInteger(argv, index, "--iterations");
        index += 1;
        break;
      case "--runs":
        config.runs = requirePositiveInteger(argv, index, "--runs");
        index += 1;
        break;
      case "--warmup":
        config.warmup = requirePositiveInteger(argv, index, "--warmup");
        index += 1;
        break;
      default:
        throw new Error(`Unknown perf option: ${arg}`);
    }
  }

  return config;
}

function requireNonEmptyValue(argv, index, flag) {
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} requires a non-empty value`);
  }

  return value;
}

function requirePositiveNumber(argv, index, flag) {
  const value = argv[index + 1];
  const parsed = Number(value);

  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${flag} must be a positive number`);
  }

  return parsed;
}

function requirePositiveInteger(argv, index, flag) {
  const value = argv[index + 1];
  const parsed = Number(value);

  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }

  return parsed;
}
