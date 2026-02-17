function bootstrap() {
  function App() {
    const label = "Klumo React Buggy Sandbox";
    return `${label}\nCount: 0`;
  }

  if (typeof document !== "undefined") {
    const root = document.getElementById("root");
    if (!root) {
      throw new Error("Missing root element");
    }
    root.textContent = App();
    return;
  }

  // Runtime fallback for non-browser execution.
  console.log(App());
}

bootstrap();
