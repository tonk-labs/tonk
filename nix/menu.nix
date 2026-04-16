# This module contains helpers for assembling and printing
# the Tonk Shell menu.
{ pkgs, ... }:
let
  tonkFlower = ''
                                  .-            
                          -##.   ..     +@@@+
                 -:    ###-  *##
                 +##+  ###* *##%
                 ########%*###*   +*%
           *###*:+##=*..:=####*++*#@
             ######*==-.----%#**##%..:                       
             .:=##-=:*#===*-##%@@*#@-                  
         =#%%%#%%***##+#-*-##@@@@@              
         =@%####@@@%%=*#@#*@@@@                -: 
           -+****#*#%@@@@%@@@-                  :.
         ++*#%*-+++*#@@@@@@@.                .:::-
         -%     =+#@   . %@@@*                    
              *#%        .%@@+                    
                           +@%+                   
                             @#=                       
                               %+-                             
                           ::: :+=-     ...:-==.
                       ......:-++:  ::.::::=+
                       ++=-::+%@@#:--:...:-=+
               .--           @@%%#*+-:::-=+-
           .=++=--=+-     -==#%%####****#-
       .%@@@      +%+ ..-*%#
       =@@@@#       --:+%-     .    +@@%  :+#%*-:.
       =@@@       ==+%  *#     +@+%@@@@ #=    %#***
     ..       .- .*=*. *        -@@#.-=#+      .**#
    +=*   ::= :..===+= #-:.     .#-...++ .=....   =
    %%.   . -+    @*---%=*@@*:  ......-=  ==...-+  +%
       .+%=*::+.*-    ##*@+--#@@@+.....=:.==. *-*+%  +%+
  '';

  makeMenu =
    commands:
    let
      names = builtins.attrNames commands;

      makeCommand =
        {
          name,
          script,
          description ? "<No description given>",
          env ? { },
        }:
        {
          inherit name description;

          package =
            with pkgs;
            writeShellApplication {
              inherit name;
              runtimeEnv = env;
              text = ''
                TITLE="$(${figlet}/bin/figlet -t '${name}')"
                SUBTITLE="${description}"

                echo "$TITLE
                $SUBTITLE
                " | ${lolcat}/bin/lolcat

                ${script}
              '';
            };
        };

      intoPackages =
        name:
        let
          element = builtins.getAttr name commands;

          task = makeCommand {
            inherit name;
            description = element.description;
            script = element.command;
            env = if builtins.hasAttr "env" element then element.env else { };
          };
        in
        task.package;

      intoLines =
        acc: name:
        let
          description = (builtins.getAttr name commands).description;
        in
        acc + " && echo '${name};${description}'";

      scripts = map intoPackages names;

      menuLines = builtins.foldl' intoLines "echo ''" names;

      menu = ''
        echo "$(${menuLines})" | column -t -s ';'
      '';
    in
    {
      header = ''
        echo "${tonkFlower}

        $(${pkgs.figlet}/bin/figlet -t "Tonk Shell")

        $(${menu})
        " | ${pkgs.lolcat}/bin/lolcat;
      '';
      menuText = ''
        echo "$(${menu})" | ${pkgs.lolcat}/bin/lolcat
      '';
      commands = scripts;
    };

  makeDevShellHook =
    { header, menuText, ... }:
    ''
      clear
      ${header}

      function showTonkMenu() {
        ${menuText}
      }

      export -f showTonkMenu
    '';

  makeMenuTestCommand = package: ''
    nix build .#${package}

    TESTS_PATH=$(nix eval .#${package}.outPath --raw)

    cargo nextest run \
      --workspace-remap ./ \
      --archive-file "$TESTS_PATH/${package}.tar.zst" \
  '';

  menuTestEnv =
    with pkgs;
    lib.optionalAttrs stdenv.isLinux {
      "CHROME" = "${chromium}/bin/chromium";
      "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
    };

  menuTestCommand =
    { description, package }:
    {
      inherit description;
      command = makeMenuTestCommand package;
      env = menuTestEnv;
    };

  # ---------------------------------------------------------------------------
  # carry-web commands
  # ---------------------------------------------------------------------------

  webBuildCommand = {
    description = "Build carry-web and install into tonk-access-service assets (fast iteration)";
    command = ''
      set -euo pipefail
      REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
      if [ -z "$REPO_ROOT" ]; then
        echo "Error: web:build must be run inside the tonk git repository." >&2
        exit 1
      fi
      cd "$REPO_ROOT"

      echo "-> Building carry-web for wasm32..."
      wasm-pack build rust/carry-web --target web --release --out-dir pkg

      echo "-> Installing WASM assets into tonk-access-service..."
      mkdir -p rust/tonk-access-service/src/assets
      install -m 644 rust/carry-web/pkg/carry_web.js \
                     rust/tonk-access-service/src/assets/carry_web.js
      install -m 644 rust/carry-web/pkg/carry_web_bg.wasm \
                     rust/tonk-access-service/src/assets/carry_web_bg.wasm

      echo "-> Done. Run 'web:dev' to serve, or rebuild with 'nix build .#tonk-access-service'."
    '';
  };

  webDevCommand = {
    description = "Run wrangler dev for tonk-access-service on localhost:8787";
    command = ''
      set -euo pipefail
      REPO_ROOT="$(git rev-parse --show-toplevel)"
      cd "$REPO_ROOT"

      # wrangler.toml's [build.command] invokes `nix build .#tonk-access-service`
      # which runs wasm-pack for carry-web and then worker-build, so we
      # don't need to pre-build anything here.
      echo "-> Starting wrangler dev on http://localhost:8787"
      echo "   Join page: http://localhost:8787/join"
      exec wrangler dev
    '';
  };

  webInviteCommand = {
    description = "Create a test space + invite URL pointing at localhost:8787/join";
    command = ''
      set -euo pipefail
      TEST_DIR="''${1:-$PWD/carry-web-test-space}"
      REPO_ROOT="$(git rev-parse --show-toplevel)"
      # The access service worker registers POST /ucan/, so the remote URL
      # must include that suffix. dialog-remote-ucan-s3 POSTs the UCAN
      # invocation directly to this URL.
      ACCESS_URL="''${WEB_ACCESS_URL:-http://localhost:8787/ucan/}"

      CARRY="cargo run --quiet --manifest-path $REPO_ROOT/Cargo.toml --bin carry --"

      if [ ! -d "$TEST_DIR/.carry" ]; then
        echo "-> Initializing test space at $TEST_DIR"
        mkdir -p "$TEST_DIR"
        (cd "$TEST_DIR" && $CARRY init "web-test")

        echo "-> Adding remote 'origin' -> $ACCESS_URL"
        (cd "$TEST_DIR" && $CARRY remote add origin "$ACCESS_URL")

        echo "-> Setting 'origin' as upstream"
        (cd "$TEST_DIR" && $CARRY remote set-upstream origin)
      fi

      echo "-> Creating invite..."
      (cd "$TEST_DIR" && $CARRY invite --url http://localhost:8787/join)
    '';
  };
in
{
  inherit
    makeMenu
    makeDevShellHook
    menuTestCommand
    webBuildCommand
    webDevCommand
    webInviteCommand
    ;
}
