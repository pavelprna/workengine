import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import createClient from "openapi-fetch";
import type { components, paths } from "./generated";

const client = createClient<paths>({
  baseUrl:
    typeof window === "undefined"
      ? "http://localhost/"
      : new URL("/", window.location.href).href,
  fetch: (...args) => globalThis.fetch(...args),
});

export const workStatuses = [
  "ready",
  "running",
  "succeeded",
  "failed",
  "parked",
] as const;

export type WorkStatus = (typeof workStatuses)[number];
export type WorkSnapshot = components["schemas"]["WorkSnapshot"];
export type WorkEvent = components["schemas"]["Event"];
export type Overview = components["schemas"]["Overview"];

export type WorkFilters = {
  status?: WorkStatus;
  profile?: string;
};

function errorMessage(error: unknown, fallback: string) {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return error.message;
  }
  return fallback;
}

async function getWorks(filters: WorkFilters, cursor?: string, limit = 50) {
  const { data, error } = await client.GET("/api/v0/works", {
    params: { query: { ...filters, cursor, limit } },
  });
  if (!data)
    throw new Error(errorMessage(error, "No response from the work queue"));
  return data;
}

async function getEvents(workId: string, after?: string) {
  const { data, error } = await client.GET("/api/v0/events", {
    params: { query: { workId, after, limit: 100 } },
  });
  if (!data)
    throw new Error(errorMessage(error, "No response from the event log"));
  return data;
}

export const api = {
  useHealth: () =>
    useQuery({
      queryKey: ["health"],
      queryFn: async () => {
        const { data, error } = await client.GET("/api/v0/health");
        if (!data)
          throw new Error(errorMessage(error, "No response from Workengine"));
        return data;
      },
    }),
  useOverview: () =>
    useQuery({
      queryKey: ["overview"],
      queryFn: async () => {
        const { data, error } = await client.GET("/api/v0/overview");
        if (!data)
          throw new Error(errorMessage(error, "No response from the overview"));
        return data;
      },
    }),
  useWorks: (filters: WorkFilters = {}, limit = 50) =>
    useInfiniteQuery({
      queryKey: ["works", filters, limit],
      initialPageParam: undefined as string | undefined,
      queryFn: ({ pageParam }) => getWorks(filters, pageParam, limit),
      getNextPageParam: (page) => page.nextCursor ?? undefined,
    }),
  useWork: (workId: string) =>
    useQuery({
      queryKey: ["work", workId],
      queryFn: async () => {
        const { data, error } = await client.GET("/api/v0/works/{workId}", {
          params: { path: { workId } },
        });
        if (!data) throw new Error(errorMessage(error, "Work was not found"));
        return data;
      },
    }),
  useEvents: (workId: string) =>
    useInfiniteQuery({
      queryKey: ["events", workId],
      initialPageParam: undefined as string | undefined,
      queryFn: ({ pageParam }) => getEvents(workId, pageParam),
      getNextPageParam: (page) => page.nextCursor ?? undefined,
    }),
  useCreateWork: () => {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: async (input: { goal: string; workerProfile: string }) => {
        const { data, error } = await client.POST("/api/v0/works", {
          body: input,
        });
        if (!data)
          throw new Error(errorMessage(error, "The task could not be created"));
        return data;
      },
      onSuccess: (work) => {
        void queryClient.invalidateQueries({ queryKey: ["overview"] });
        void queryClient.invalidateQueries({ queryKey: ["works"] });
        queryClient.setQueryData(["work", work.workId], work);
      },
    });
  },
  useStartWork: () => {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: async (workId: string) => {
        const { data, error } = await client.POST(
          "/api/v0/works/{workId}/start",
          { params: { path: { workId } } },
        );
        if (!data)
          throw new Error(errorMessage(error, "The task could not be started"));
        return data;
      },
      onSuccess: (work) => {
        queryClient.setQueryData(["work", work.workId], work);
        void queryClient.invalidateQueries({ queryKey: ["overview"] });
        void queryClient.invalidateQueries({ queryKey: ["works"] });
        void queryClient.invalidateQueries({
          queryKey: ["events", work.workId],
        });
      },
    });
  },
};
