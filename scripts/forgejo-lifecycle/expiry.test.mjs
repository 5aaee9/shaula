import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { expiry } from "./expiry.mjs";

test("hard expiry restarts only after the failed Destroy is durable", async () => {
  let destroyPending = false;
  let checkpoint;
  let busyRestarted = false;
  const fixture = {
    prefix: "expiry-regression", platform: "docker",
    registrationProxy: { gate: { failures: 0 } }, dockerProxy: { gate: { failures: 0 } },
    async restart() {
      if (this.dockerProxy.gate.failures) {
        assert(destroyPending, "restart interrupted Destroy before its failure checkpoint");
        busyRestarted = true;
      } else if (this.registrationProxy.gate.block) {
        this.registrationProxy.gate.failures++;
      }
    },
    async createFleet() {},
    async queue() {},
    async observe(key) { return { id: key, runner: 1 }; },
    async capZero(key) {
      if (key.endsWith("-idle")) {
        this.registrationProxy.gate.failures++;
      } else {
        // The HTTP failure is visible before Terraform exits and the daemon
        // commits DestroyPending, just as in the failing hosted run.
        this.dockerProxy.gate.failures++;
        checkpoint = delay(500).then(() => { destroyPending = true; });
      }
    },
    async api(path) {
      if (path.startsWith("/generations?")) {
        return { items: [{ state: destroyPending ? "DestroyPending" : "Destroying" }] };
      }
      return { capacity: { occupancy: 1 } };
    },
    async cli() { return `${this.prefix}-busy`; },
    async forgejo() { return [{ id: 1 }]; },
    async reclaimed(key) {
      if (key.endsWith("-busy")) assert(busyRestarted && !this.dockerProxy.gate.block);
    },
  };
  const report = {};
  try {
    await expiry(fixture, report);
    assert.equal(report.destroyRetryAcrossRestart, "passed");
  } finally {
    await checkpoint;
  }
});
