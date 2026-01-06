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

  makeMenu = commands:
    let
      names =
        builtins.attrNames commands;

      makeCommand = { name, script, description ? "<No description given>", env ? { } }:
        {
          inherit name description;

          package = with pkgs; writeShellApplication
            {
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

      intoPackages = name:
        let
          element =
            builtins.getAttr name commands;

          task = makeCommand {
            inherit name;
            description = element.description;
            script = element.command;
            env =
              if builtins.hasAttr "env" element
              then element.env
              else { };
          };
        in
        task.package;

      intoLines = acc: name:
        let
          description = (builtins.getAttr name commands).description;
        in
        acc + " && echo '${name};${description}'";

      scripts =
        map intoPackages names;

      menuLines = builtins.foldl' intoLines "echo ''" names;

      menu = ''
        echo "$(${menuLines})" | column -t --s ';'
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
in
{
  makeMenu = makeMenu;
}
