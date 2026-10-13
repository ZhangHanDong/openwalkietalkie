import CryptoKit
import Foundation

// The secret travels in a protected file, never as a command-line argument.
guard CommandLine.arguments.count == 2,
      let text = try? String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8),
      let seed = Data(base64Encoded: text.trimmingCharacters(in: .whitespacesAndNewlines)),
      seed.count == 32 else {
    fputs("Expected a file containing a base64 Ed25519 seed\n", stderr)
    exit(1)
}
let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
print(key.publicKey.rawRepresentation.base64EncodedString())
