process.stdout.write("fixture ready\n");
const timer = setInterval(() => {}, 1_000);
process.on("SIGTERM", () => {
  clearInterval(timer);
  process.exit(0);
});
