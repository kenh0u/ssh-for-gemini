{ lib
, rustPlatform
, doCheck ? true
, version ? "0.0.0"
}:

rustPlatform.buildRustPackage {
  pname = "ssh-for-gemini";
  inherit version;

  src = lib.cleanSource ./.;
  inherit doCheck;

  cargoLock.lockFile = ./Cargo.lock;

  meta = with lib; {
    description = "SSH for Gemini: pure HTTP POST/GET <-> local TCP (SSH) bridge daemon";
    homepage = "https://github.com/kenh0u/ssh-for-gemini";
    license = with licenses; [ mit asl20 ];
    mainProgram = "ssh-for-gemini";
  };
}
