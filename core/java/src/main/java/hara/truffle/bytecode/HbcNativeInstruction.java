package hara.truffle.bytecode;

/** Immutable instruction identity embedded in generated Truffle operations. */
public record HbcNativeInstruction(HbcProgram program, int functionIndex, int instructionPointer) {}
