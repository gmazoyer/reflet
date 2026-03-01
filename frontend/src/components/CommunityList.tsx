import type {
  Community,
  ExtCommunity,
  LargeCommunity,
  CommunityDefinitions,
} from "../api/types";
import { annotateStandardCommunity, annotateLargeCommunity } from "../utils/communityAnnotation";
import CommunityBadge from "./CommunityBadge";

interface CommunityListProps {
  communities: Community[];
  extCommunities: ExtCommunity[];
  largeCommunities: LargeCommunity[];
  definitions?: CommunityDefinitions;
  onFilterByCommunity?: (filter: string) => void;
}

function formatExtCommunity(ec: ExtCommunity): string {
  const hex = ec.value.map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${ec.type_high.toString(16).padStart(2, "0")}:${ec.type_low.toString(16).padStart(2, "0")}:${hex}`;
}

export default function CommunityList({
  communities,
  extCommunities,
  largeCommunities,
  definitions,
  onFilterByCommunity,
}: CommunityListProps) {
  const hasAny =
    communities.length > 0 ||
    extCommunities.length > 0 ||
    largeCommunities.length > 0;

  if (!hasAny) {
    return <span className="text-gray-400 dark:text-gray-500">-</span>;
  }

  return (
    <span className="inline-flex gap-1.5">
      {communities.map((c, i) => {
        const desc = definitions
          ? annotateStandardCommunity(c, definitions)
          : null;
        return (
          <CommunityBadge
            key={`std-${i}`}
            value={`${c.asn}:${c.value}`}
            description={desc}
            onClick={onFilterByCommunity ? () => onFilterByCommunity(`community:${c.asn}:${c.value}`) : undefined}
          />
        );
      })}
      {largeCommunities.map((c, i) => {
        const desc = definitions
          ? annotateLargeCommunity(c, definitions)
          : null;
        return (
          <CommunityBadge
            key={`large-${i}`}
            value={`${c.global_admin}:${c.local_data1}:${c.local_data2}`}
            description={desc}
            onClick={onFilterByCommunity ? () => onFilterByCommunity(`lc:${c.global_admin}:${c.local_data1}:${c.local_data2}`) : undefined}
          />
        );
      })}
      {extCommunities.map((ec, i) => (
        <CommunityBadge
          key={`ext-${i}`}
          value={formatExtCommunity(ec)}
        />
      ))}
    </span>
  );
}
