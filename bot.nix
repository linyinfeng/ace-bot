{ rustPlatform, lib, pkg-config, openssl }:

rustPlatform.buildRustPackage {
  pname = "ace-bot";
  version = "0.0.1";
  src = ./bot;
  cargoLock.lockFile = ./bot/Cargo.lock;

  nativeBuildInputs = [
    pkg-config
  ];
  buildInputs = [
    openssl
  ];

  meta = with lib; {
    homepage = "https://github.com/linyinfeng/ace-bot";
    license = licenses.mit;
  };
}
