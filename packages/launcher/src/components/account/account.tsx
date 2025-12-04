import { useMemo } from 'react';
import AccountDialog from './accountDialog';

export default function Account() {
  const base = useMemo(() => {
    return (
      <>
        <div>This is your settings</div>
        <AccountDialog.Footer>
          <AccountDialog.Close />
        </AccountDialog.Footer>
      </>
    );
  }, []);

  return <AccountDialog.Root title="Account">{base}</AccountDialog.Root>;
}
