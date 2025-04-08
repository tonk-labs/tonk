import React, {createContext, useContext} from 'react';
import {createTRPCClient} from '@trpc/client';
import type {CreateTRPCClientOptions} from '@trpc/client';
import {AnyRouter} from '@trpc/server';
import type {TRPCClientError} from '@trpc/client';

export function createTonkRPC<TRouter extends AnyRouter>(
  config: CreateTRPCClientOptions<TRouter>,
) {
  const client = createTRPCClient<TRouter>(config);
  const TRPCContext = createContext(client);
  const TonkProvider: React.FC<{children: React.ReactNode}> = ({children}) => {
    return (
      <TRPCContext.Provider value={client}>{children}</TRPCContext.Provider>
    );
  };

  const useTonk = () => {
    const context = useContext(TRPCContext);
    if (!context) {
      throw new Error('useTRPC must be used within a TRPCProvider');
    }
    return context;
  };

  return {
    TonkProvider,
    useTonk,
    client,
  };
}
