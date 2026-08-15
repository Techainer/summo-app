import { boot } from "./daemon.mjs";
const engine = await boot({ name: "shots" });
console.log(`URL=${engine.url} TOKEN=${engine.token}`);
setInterval(() => {}, 1 << 30);
