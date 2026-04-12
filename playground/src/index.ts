import { serve } from "bun";
import index from "./index.html";
import wasm1 from "generated/jsongrep_wasm.core.wasm";
import wasm2 from "generated/jsongrep_wasm.core2.wasm";

const server = serve({
  routes: {
    // Serve index.html for all unmatched routes.
    "/": index,
    "/jsongrep_wasm.core.wasm": () => {
      return new Response(Bun.file(wasm1));
    },
    "/jsongrep_wasm.core2.wasm": () => {
      return new Response(Bun.file(wasm2));
    },
  },
  development: process.env.NODE_ENV !== "production" && {
    // Enable browser hot reloading in development
    hmr: false,

    // Echo console logs from the browser to the server
    console: true,
  },
});

console.log(`🚀 Server running at ${server.url}`);
