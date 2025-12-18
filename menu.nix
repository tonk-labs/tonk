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
        }:
        {
          inherit name description;

          package = pkgs.writeScriptBin name ''
            #!${pkgs.bash}/bin/bash
            ${pkgs.figlet}/bin/figlet '${name}' | ${pkgs.lolcat}/bin/lolcat
            ${script}
          '';
        };

      intoPackages =
        name:
        let
          element = builtins.getAttr name commands;

          task = makeCommand {
            inherit name;
            description = element.description;
            script = element.command;
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
        echo "$(${menuLines})"  | column -t -s ';'
      '';
    in
    {
      header = ''
        echo "${tonkFlower}

        $(${pkgs.figlet}/bin/figlet "Tonk Shell")

        $(${menu})
        " | ${pkgs.lolcat}/bin/lolcat;
      '';
      commands = scripts;
    };
in
{
  makeMenu = makeMenu;
}
