import App from "./App.jsx"

function bootstrap() {
  const root = document.getElementById("root");
  if (!root) {
    throw new Error("Missing root element");
  }

  root.textContent = App();
}

bootstrap(
bootstrap();
