# This module contains helpers for assembling and printing
# the Tonk Shell menu.
{
  pkgs,
  # chromedriver for macOS test commands; null on Linux (see flake.nix).
  chromedriverDarwin ? null,
  ...
}:
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

  makeMenuTestCommand =
    {
      package,
      runner ? null,
      clearPoolEnv ? false,
    }:
    ''
        ${pkgs.lib.optionalString clearPoolEnv ''
          unset \
            WBG_POOL_BROWSER \
            WBG_POOL_BROWSER_ARGS \
            WBG_POOL_DIR \
            WBG_POOL_FALLBACK_RUNNER \
            WBG_POOL_NO_SANDBOX \
            WBG_POOL_URL
        ''}

          ${pkgs.lib.optionalString (runner != null) ''
            export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=${pkgs.lib.escapeShellArg runner}
          ''}

        nix build .#${package}

        TESTS_PATH=$(nix eval .#${package}.outPath --raw)

      cargo nextest run \
        --workspace-remap ./ \
        --archive-file "$TESTS_PATH/${package}.tar.zst" \
        "$@"
    '';

  menuTestEnv =
    with pkgs;
    lib.optionalAttrs stdenv.isLinux {
      "CHROME" = "${chromium}/bin/chromium";
      "CHROMEDRIVER" = "${chromedriver}/bin/chromedriver";
    }
    // lib.optionalAttrs stdenv.isDarwin {
      "CHROMEDRIVER" = "${chromedriverDarwin}/bin/chromedriver";
    };

  menuTestCommand =
    {
      description,
      package,
      runner ? null,
      clearPoolEnv ? false,
    }:
    {
      inherit description;
      command = makeMenuTestCommand { inherit package runner clearPoolEnv; };
      env = menuTestEnv;
    };
in
{
  inherit makeMenu makeDevShellHook menuTestCommand;
}
