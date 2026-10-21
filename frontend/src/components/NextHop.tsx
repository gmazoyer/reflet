import type { BgpRoute } from "../api/types";
import { displayAddress } from "../utils/hiddenAddress";
import { describeNextHop } from "../utils/nextHop";
import Tooltip from "./Tooltip";

interface NextHopProps {
  route: BgpRoute;
  /** Show the address family and next hop type below the address. */
  showDetails?: boolean;
}

/** The next hop of a route, with its link-local next hop when there is one. */
export default function NextHop({ route, showDetails }: NextHopProps) {
  const info = describeNextHop(route);
  const address = <span className={showDetails ? undefined : "cursor-help"}>{displayAddress(route.next_hop)}</span>;

  return (
    <div className="font-mono text-sm text-gray-600 dark:text-gray-400">
      <div className="flex items-center gap-2">
        {showDetails ? (
          address
        ) : (
          <Tooltip
            content={
              <>
                <div>{info.afiSafi}</div>
                <div>Next hop: {info.type}</div>
              </>
            }
          >
            {address}
          </Tooltip>
        )}
        {route.next_hop_link_local && (
          <Tooltip content="Link-local next hop">
            <span className="cursor-help text-gray-400 dark:text-gray-500">
              {displayAddress(route.next_hop_link_local)}
            </span>
          </Tooltip>
        )}
      </div>
      {showDetails && (
        <div className="font-sans text-xs text-gray-500 dark:text-gray-500">
          {info.afiSafi} · {info.type}
        </div>
      )}
    </div>
  );
}
