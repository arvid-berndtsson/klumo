import isChar from "jsr:@arvid/is-char";

function bootstrap() {
  function App() {
    const label = "Beeno React Buggy Sandbox";
    const probe = isChar("B") ? "B is a char" : "B is not a char";
    return `${label}\nCount: 0\n${probe}`;
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
