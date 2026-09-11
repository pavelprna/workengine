import { useQuery } from "@tanstack/react-query";
import createClient from "openapi-fetch";
import type { paths } from "./generated";

const client = createClient<paths>({ baseUrl: "/" });

export const api = {
  useHealth: () =>
    useQuery({
      queryKey: ["health"],
      queryFn: async () => {
        const { data, error } = await client.GET("/api/v0/health");
        if (error || !data) throw new Error("No response from the observer");
        return data;
      },
    }),
};
