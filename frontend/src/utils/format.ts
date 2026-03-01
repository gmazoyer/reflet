/** Map BGP origin code to its short display label. */
export function formatOrigin(origin: string): { label: string; description: string } {
  switch (origin) {
    case "IGP":
      return { label: "i", description: "IGP" };
    case "EGP":
      return { label: "e", description: "EGP" };
    default:
      return { label: "?", description: "Incomplete" };
  }
}
