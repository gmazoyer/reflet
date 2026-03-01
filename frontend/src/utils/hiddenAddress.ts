/** Returns true when the address was masked by the server (hide_peer_addresses). */
export function isHiddenAddress(addr: string): boolean {
  return addr === "0.0.0.0" || addr === "::";
}

/** Display a masked address as "Hidden", pass through otherwise. */
export function displayAddress(addr: string): string {
  return isHiddenAddress(addr) ? "Hidden" : addr;
}
