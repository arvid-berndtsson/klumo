import isChar from "jsr:@arvid/is-char";

const single = isChar("B");
const multi = isChar("be");

console.log(`isChar(B)=${single}`);
console.log(`isChar(be)=${multi}`);

if (!single || multi) {
  throw new Error("JSR is-char behavior did not match expected values");
}

console.log("JSR_IS_CHAR_OK");
