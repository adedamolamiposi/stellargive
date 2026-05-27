"use client";

import { useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRecentCampaigns } from "@/hooks/useSoroban";
import { CampaignCard } from "@/components/CampaignCard";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Search } from "lucide-react";

export function CampaignList() {
  const { data: campaigns, isLoading, error } = useRecentCampaigns();
  const [searchTerm, setSearchTerm] = useState("");
  const [debouncedSearchTerm, setDebouncedSearchTerm] = useState("");

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      setDebouncedSearchTerm(searchTerm);
    }, 300);

    return () => window.clearTimeout(timeout);
  }, [searchTerm]);

  const filteredCampaigns = useMemo(() => {
    const term = debouncedSearchTerm.trim().toLowerCase();
    const campaignList = campaigns ?? [];

    if (!term) {
      return campaignList;
    }

    return campaignList.filter(
      (campaign) =>
        campaign.title.toLowerCase().includes(term) ||
        campaign.creator.toLowerCase().includes(term)
    );
  }, [campaigns, debouncedSearchTerm]);

  if (isLoading) {
    return (
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {[1, 2, 3].map((i) => (
          <div key={i} className="h-[300px] rounded-xl bg-muted animate-pulse" />
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <div className="text-center py-12 text-destructive">
        Failed to load campaigns. Please ensure you are on Testnet.
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h2 className="text-2xl font-semibold tracking-tight">Campaigns</h2>
          <p className="text-sm text-muted-foreground">
            Search by campaign name or creator address.
          </p>
        </div>
        <div className="relative w-full sm:max-w-sm">
          <label htmlFor="campaign-search" className="sr-only">
            Search campaigns
          </label>
          <Search
            className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            id="campaign-search"
            type="search"
            value={searchTerm}
            onChange={(event) => setSearchTerm(event.target.value)}
            placeholder="Search campaigns"
            autoComplete="off"
            className="pl-9"
          />
        </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {filteredCampaigns.map((campaign) => (
          <CampaignCard key={campaign.id.toString()} campaign={campaign} />
        ))}
        {(campaigns?.length ?? 0) === 0 && (
          <div className="col-span-full text-center py-12 text-muted-foreground">
            No campaigns found. Be the first to create one!
          </div>
        )}
        {(campaigns?.length ?? 0) > 0 && filteredCampaigns.length === 0 && (
          <div className="col-span-full flex flex-col items-center gap-4 py-12 text-center">
            <div>
              <p className="font-medium text-foreground">No results found</p>
              <p className="text-sm text-muted-foreground">
                Clear your search or create a new campaign.
              </p>
            </div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <Button variant="outline" onClick={() => setSearchTerm("")}>
                Clear search
              </Button>
              <Button asChild>
                <Link href="/create">Create campaign</Link>
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
