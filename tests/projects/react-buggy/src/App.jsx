import { useState } from "react";

export default function App() {
  const [count, setCount] = useState(0);
  const label = "Klumo React Buggy Sandbox";

  return (
    <main style={{ fontFamily: "sans-serif", padding: 24 }}>
      <h1>{label}</h1>
      <p>Count: {count}</p>
      <button onClick={() => setCount(count + 1)}>Increment</button>
    </main>
  );
}
