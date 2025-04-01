import { create } from "zustand";

interface LoginState {
  loggedIn: boolean;
  username: string;
  login: (username: string) => void;
  logout: () => void;
  getUsername: () => string;
}

const useLoginState = create<LoginState>((set, get) => ({
  loggedIn: false,
  username: "",

  login: (username: string) =>
    set(() => ({
      loggedIn: true,
      username,
    })),

  logout: () =>
    set(() => ({
      loggedIn: false,
      username: "",
    })),

  getUsername: () => get().username,
}));

export default useLoginState;
