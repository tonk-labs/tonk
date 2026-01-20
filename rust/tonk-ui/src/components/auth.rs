use leptos::prelude::*;
use tonk_worker::{AuthorizeRequest, AuthorizeResponse};

use crate::{components::Status, error::TonkUiError};

/// Authentication form for user login with R2 credentials.
#[component]
pub fn TonkAuth() -> impl IntoView {
    let (access_key_id, set_access_key_id) = signal_local(String::new());
    let (secret_access_key, set_secret_access_key) = signal_local(String::new());

    let status = use_context::<Signal<Status, LocalStorage>>().expect("Missing status");

    let authorize_action =
        use_context::<Action<AuthorizeRequest, Result<AuthorizeResponse, TonkUiError>>>()
            .expect("Missing authorize action");

    let is_authorizing = move || status.get() == Status::Authorizing;

    let ready_to_submit = move || {
        status.get() == Status::Unauthorized
            && !access_key_id.get().is_empty()
            && !secret_access_key.get().is_empty()
    };

    let submit = move |_| {
        authorize_action.dispatch(AuthorizeRequest {
            access_key_id: access_key_id.get(),
            secret_access_key: secret_access_key.get(),
        });
    };

    let button_text = move || {
        if is_authorizing() {
            "Authorizing..."
        } else {
            "Authorize"
        }
    };

    // Show form when unauthorized or authorizing
    let show_form = move || matches!(status.get(), Status::Unauthorized | Status::Authorizing);

    view! {
        <section
            class="auth"
            class:pending=show_form
        >
            <section
                class="loading-indicator"
                class:visible=is_authorizing
            >
            </section>
            <section
                class="panel"
            >
                <input
                    type="text"
                    placeholder="Access Key ID"
                    prop:value=access_key_id
                    disabled=is_authorizing
                    on:input=move |ev| {
                        set_access_key_id.set(event_target_value(&ev));
                    }
                />
                <input
                    type="password"
                    placeholder="Secret Access Key"
                    prop:value=secret_access_key
                    disabled=is_authorizing
                    on:input=move |ev| {
                        set_secret_access_key.set(event_target_value(&ev));
                    }
                />
                <button
                    aria-label=button_text
                    disabled=move || !ready_to_submit()
                    on:click=submit
                >{button_text}</button>
            </section>
        </section>
    }
}

#[cfg(all(
    test,
    not(any(target_arch = "wasm32", feature = "web-integration-tests"))
))]
mod integration_tests {
    #![allow(unexpected_cfgs)]

    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use crate::helpers::TestEnvironment;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use anyhow::Result;
    #[cfg_attr(not(feature = "integration-tests"), allow(unused))]
    use thirtyfour::prelude::*;

    #[dialog_common::test]
    async fn it_reaches_the_website(test_environment: TestEnvironment) -> Result<()> {
        let driver = test_environment.driver().await?;
        let title = driver.title().await?;

        assert_eq!(title, "Tonk");

        driver.quit().await?;

        Ok(())
    }

    #[dialog_common::test]
    async fn it_authorizes_a_session_with_credentials_and_shows_logged_in_view(
        test_environment: TestEnvironment,
    ) -> Result<()> {
        let driver = test_environment.driver().await?;

        let access_key_id_input = driver
            .query(By::Css("[placeholder='Access Key ID']"))
            .first()
            .await?;
        access_key_id_input
            .send_keys("AKIAIOSFODNN7EXAMPLE")
            .await?;

        let secret_access_key_input = driver
            .find(By::Css("[placeholder='Secret Access Key']"))
            .await?;
        secret_access_key_input
            .send_keys("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")
            .await?;

        let authorize_button = driver
            .query(By::Css("[aria-label='Authorize']:not(:disabled)"))
            .first()
            .await?;
        authorize_button.click().await?;

        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        driver.quit().await?;

        Ok(())
    }
}
