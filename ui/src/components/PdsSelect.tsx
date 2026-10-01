import { Select, SelectItem } from "@heroui/react";
import { useQuery } from "@tanstack/react-query";
import { useAtom } from "jotai";
import { pdsUrlAtom } from "../atoms/store";
import { fetchKnownServers, hostOf } from "../lib/servers";

/// Picks which PDS the app talks to.
///
/// The list comes from the gateway, so it names the fleet without hardcoding it.
/// A server the user reached by detection but that is not in the list is added,
/// so the current selection is always visible.
export function PdsSelect({ label = "Server" }: { label?: string }) {
  const [pds, setPds] = useAtom(pdsUrlAtom);

  const { data: servers = [] } = useQuery({
    queryKey: ["knownServers"],
    staleTime: 5 * 60 * 1000,
    queryFn: () => fetchKnownServers(),
  });

  const options = servers.some((s) => s.url === pds)
    ? servers
    : [...servers, { name: "", url: pds }];

  return (
    <Select
      label={label}
      variant="bordered"
      selectedKeys={[pds]}
      disallowEmptySelection
      onSelectionChange={(keys) => {
        const next = Array.from(keys)[0];
        if (typeof next === "string") setPds(next);
      }}
    >
      {options.map((server) => (
        <SelectItem key={server.url} textValue={hostOf(server.url)}>
          {hostOf(server.url)}
        </SelectItem>
      ))}
    </Select>
  );
}
