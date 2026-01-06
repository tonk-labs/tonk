use leptos::prelude::*;
use tonk_worker::{AuthorizeRequest, AuthorizeResponse};

use crate::error::TonkUiError;

/// Authentication form for user login with account credentials.
#[component]
pub fn TonkAuth() -> impl IntoView {
    let (is_submitted, set_is_submitted) = signal_local(false);
    let (account_id, set_account_id) = signal_local(String::new());
    let (secret_key, set_secret_key) = signal_local(String::new());

    let authorize_action =
        use_context::<Action<AuthorizeRequest, Result<AuthorizeResponse, TonkUiError>>>()
            .expect("Missing expected authorize action");

    let authorization = use_context::<Signal<Option<AuthorizeResponse>, LocalStorage>>()
        .expect("Missing expected authorization signal");

    let ready_to_submit =
        move || !account_id.get().is_empty() && !secret_key.get().is_empty() && !is_submitted.get();

    Effect::new(move |_| {
        if !is_submitted.get() {
            return;
        }

        let account_id = account_id.get();
        let secret_key = secret_key.get();

        authorize_action.dispatch(AuthorizeRequest {
            secret_key,
            account_id,
        });
    });

    let button_text = move || {
        if is_submitted.get() {
            "Authorizing..."
        } else {
            "Authorize"
        }
    };

    view! {
        <section
            class="auth"
            class:pending=move || authorization.get().is_none()
        >
            <section
                class="loading-indicator"
                class:visible=move || is_submitted.get() && authorization.get().is_none()
            >
            </section>
            <section
                class="panel"
            >
                <input
                    type="text"
                    placeholder="Account ID"
                    prop:value=account_id
                    disabled=is_submitted
                    on:input=move |ev| {
                        set_account_id.set(event_target_value(&ev));
                    }
                />
                <input
                    type="text"
                    placeholder="Secret Key"
                    prop:value=secret_key
                    disabled=is_submitted
                    on:input=move |ev| {
                        set_secret_key.set(event_target_value(&ev));
                    }
                />
                <button
                    aria-label=button_text
                    disabled=move || !ready_to_submit()
                    on:click=move |_| set_is_submitted.set(true)
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

        let account_id_input = driver
            .query(By::Css("[placeholder='Account ID']"))
            .first()
            .await?;
        account_id_input.send_keys("abc123").await?;

        let secret_key_input = driver.find(By::Css("[placeholder='Secret Key']")).await?;
        secret_key_input.send_keys("123abc").await?;

        let authorize_button = driver.find(By::Css("[aria-label='Authorize']")).await?;
        authorize_button.click().await?;

        assert!(driver.query(By::Css(".toolbar.visible")).exists().await?);

        driver.quit().await?;

        Ok(())
    }
}
