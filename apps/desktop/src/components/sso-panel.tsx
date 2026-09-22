/** Selects the sign-in or sign-up UI for the shared SSO flow. */

import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type { PanelPresentation } from './panel-sheet';
import { SsoSignInSheet } from './sso/sign-in-sheet';
import { SsoSignUpPanel } from './sso/sign-up-panel';
import type { SsoSignUpPanelProps } from './sso/sign-up-panel';

export { SsoSignInSheet } from './sso/sign-in-sheet';
export { SsoSignUpPanel } from './sso/sign-up-panel';

interface SignUpProps extends SsoSignUpPanelProps {
  login: false;
}

interface SignInProps {
  login: true;
  bridge: Bridge;
  profile: string;
  account: string;
  onComplete: () => void | Promise<void>;
  /** Sheet title, subtitle, and close handler. */
  presentation: PanelPresentation;
}

export function SsoPanel(props: SignInProps | SignUpProps): ReactNode {
  if (props.login) {
    const { bridge, profile, account, presentation, onComplete } = props;
    return (
      <SsoSignInSheet
        bridge={bridge}
        profile={profile}
        account={account}
        presentation={presentation}
        onComplete={onComplete}
      />
    );
  }
  return <SsoSignUpPanel {...props} />;
}
