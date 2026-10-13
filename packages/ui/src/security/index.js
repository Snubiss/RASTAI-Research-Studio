// @rastai/ui/security: the locked security UI (owner decision N7; layout frontend-and-plugins.md 2.10). Only the
// app shell may import this subpath (check-imports allows it under apps/web/src/ alone), so plugin UIs can't render
// these components and can't fake them. They are drawn with locked tokens only.

export { EgressBadge } from "./EgressBadge.jsx";
export { PermissionList } from "./PermissionList.jsx";
export { SignedOutScreen } from "./SignedOutScreen.jsx";
export { TrustPrompt } from "./TrustPrompt.jsx";

/** @typedef {import("./EgressBadge.jsx").ActiveEgress} ActiveEgress */
