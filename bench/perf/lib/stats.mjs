export function median(values) {
  const sorted = sortedNumbers(values);
  if (sorted.length === 0) {
    throw new Error("Cannot calculate median of an empty array");
  }

  const middle = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) {
    return sorted[middle];
  }

  return (sorted[middle - 1] + sorted[middle]) / 2;
}

export function percentile(values, p) {
  const sorted = sortedNumbers(values);
  if (sorted.length === 0) {
    throw new Error("Cannot calculate percentile of an empty array");
  }
  if (p < 0 || p > 1) {
    throw new Error("Percentile must be between 0 and 1");
  }

  const index = Math.ceil(p * sorted.length) - 1;
  return sorted[Math.max(0, Math.min(sorted.length - 1, index))];
}

export function relativeToFastest(results) {
  const fastest = Math.min(...results.map((result) => result.medianSeconds));

  return results.map((result) => ({
    ...result,
    relativeToFastest: result.medianSeconds / fastest
  }));
}

function sortedNumbers(values) {
  return [...values].sort((left, right) => left - right);
}
