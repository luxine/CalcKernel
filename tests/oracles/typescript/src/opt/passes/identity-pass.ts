import type { MirPass } from "../mir-pass.js";

export const identityPass: MirPass = {
  name: "identity",
  run() {
    return { changed: false };
  }
};
