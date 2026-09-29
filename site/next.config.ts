import type {NextConfig} from "next";

const nextConfig: NextConfig = {
  // Logs are only read and written through the API, never exposed directly.
  poweredByHeader: false,
  async headers() {
    return [
      {
        source: "/(.*)",
        headers: [
          {key: "X-Content-Type-Options", value: "nosniff"},
          {key: "X-Frame-Options", value: "DENY"},
          {key: "Referrer-Policy", value: "no-referrer"},
        ],
      },
      {
        // Shared logs aren't for search engines.
        source: "/l/:id",
        headers: [{key: "X-Robots-Tag", value: "noindex, nofollow"}],
      },
    ];
  },
};

export default nextConfig;
