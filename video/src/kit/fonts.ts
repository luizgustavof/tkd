import { loadFont as inter } from "@remotion/google-fonts/Inter";
import { loadFont as anton } from "@remotion/google-fonts/Anton";
import { loadFont as bebas } from "@remotion/google-fonts/BebasNeue";
import { loadFont as poppins } from "@remotion/google-fonts/Poppins";
import { loadFont as playfair } from "@remotion/google-fonts/PlayfairDisplay";

type Loader = (style?: any, options?: any) => { fontFamily: string };

const loaders: Record<string, Loader> = {
  Inter: inter,
  Anton: anton,
  BebasNeue: bebas,
  Poppins: poppins,
  PlayfairDisplay: playfair,
};

export type FontName = "Inter" | "Anton" | "BebasNeue" | "Poppins" | "PlayfairDisplay";

// latin-ext garante os acentos do pt-BR (CAO, VOCE -> ÇÃO, VOCÊ)
export const fontFamily = (name: FontName): string =>
  (loaders[name] ?? loaders.Inter)(undefined, {
    subsets: ["latin", "latin-ext"],
  }).fontFamily;
