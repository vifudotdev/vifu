import {
  VIFU_PROTOCOL_VERSION,
  VIFU_SDK_VERSION,
} from "./constants.js";
import { createVifuSDK } from "./sdk.js";

const root = typeof window !== "undefined" ? window : globalThis;

const existing = typeof root.vifu?.invoke === "function"
  ? root.vifu
  : typeof root.Vifu?.invoke === "function"
    ? root.Vifu
    : null;

const vifu = existing || createVifuSDK({
  transport: "auto",
  documentTitle: root.document?.title || "vifu-game",
});

vifu.version = VIFU_SDK_VERSION;
vifu.protocolVersion = VIFU_PROTOCOL_VERSION;
vifu.__receiveHostMessage = (envelopeOrMessage) => vifu._handleEnvelope(envelopeOrMessage);

root.vifu = vifu;
root.Vifu = vifu;
