import React from "react";
import { Box, Text } from "ink";

import AsyncButton from "../components/AsyncButton";
import { InputBox } from "../components";
import { useForm, useSelection } from "../hooks";
import { ROUTE, useNavigate, useRouteData } from "../routes";
import { useLoginState } from "../store";

export const Login: React.FC = () => {
  const navigate = useNavigate();
  const { username } = useRouteData<ROUTE.LOGIN>();
  const { login } = useLoginState();
  const { data, register } = useForm({
    initialValues: {
      password: "",
    },
  });

  const { selection } = useSelection({
    amount: 3,
    prevKey: "upArrow",
    nextKey: ["downArrow", "return"],
    isActive: true,
  });

  // const { execute, isLoading, error } = useAsync(async () => {
  //   do some async stuff here
  // });

  const loginUser = () => {
    console.log(data.password);
    login(username);
    navigate(ROUTE.HOME);
  };

  return (
    <Box flexDirection="column" width="50%">
      <Box justifyContent="center" marginBottom={1}>
        <Text>"Sample Login Page"</Text>
      </Box>
      <InputBox
        label="password"
        mask="*"
        focus={selection === 0}
        {...register("password")}
      />
      <AsyncButton
        isFocused={selection === 1}
        onPress={loginUser}
        spinner="fingerDance"
      >
        Unlock
      </AsyncButton>
    </Box>
  );
};
