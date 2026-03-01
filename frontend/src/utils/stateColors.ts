/** BGP session state → Tailwind badge classes. Shared by PeerCard and PeerList. */
export const stateColors: Record<string, string> = {
  Established: "bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400",
  Active: "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400",
  Connect: "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400",
  OpenSent: "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400",
  OpenConfirm: "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400",
  Idle: "bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400",
};

export const defaultStateColor =
  "bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-300";
