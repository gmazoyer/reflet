import type { AsPathSegment, AsnMap } from "../api/types";
import Tooltip from "./Tooltip";

interface AsPathProps {
  segments: AsPathSegment[];
  asnInfo?: AsnMap;
}

export default function AsPath({ segments, asnInfo }: AsPathProps) {
  if (segments.length === 0) {
    return <span className="text-gray-400 dark:text-gray-500">-</span>;
  }

  return (
    <span className="font-mono text-sm">
      {segments.map((seg, i) => (
        <span key={i}>
          {seg.type === "Set" && "{ "}
          {seg.asns.map((asn, j) => {
            const info = asnInfo?.[String(asn)];
            const link = (
              <a
                href={`https://bgp.tools/as/${asn}`}
                target="_blank"
                rel="noopener noreferrer"
                className="text-blue-700 dark:text-blue-400 hover:underline"
              >
                {asn}
              </a>
            );
            return (
              <span key={j}>
                {info ? (
                  <Tooltip
                    content={<>AS{asn} &mdash; {info.name}</>}
                  >
                    {link}
                  </Tooltip>
                ) : (
                  link
                )}
                {j < seg.asns.length - 1 && (seg.type === "Set" ? ", " : " ")}
              </span>
            );
          })}
          {seg.type === "Set" && " }"}
          {i < segments.length - 1 && " "}
        </span>
      ))}
    </span>
  );
}
