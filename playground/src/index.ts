import { serve } from "bun";
import index from "./index.html";
import wasm1 from "generated/jsongrep_wasm.core.wasm";

const server = serve({
  routes: {
    "/": index,
    "/jsongrep_wasm.core.wasm": () => {
      return new Response(Bun.file(wasm1));
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
