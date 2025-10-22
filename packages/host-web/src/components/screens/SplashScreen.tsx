import icon_tonk from "../../assets/icon-tonk.svg";

export function SplashScreen() {
  return (
    <div class="flex min-h-0 h-full w-full grow items-center justify-center bg-[#E0E0E0]">
        <img src={icon_tonk} alt="tonk icon" class="w-8 h-8 mx-auto"/>
    </div>
  );
}
