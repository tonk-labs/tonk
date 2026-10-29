//! Real-browser tests of the kernels sites pin: a space runs the kernel it
//! was created with, and the profile the persona it was created with,
//! whichever ones the deployment ships after.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::time::Duration;

    use anyhow::{Context, Result, ensure};
    use serde::Deserialize;
    use serde_json::Value;
    use thirtyfour::prelude::*;

    use crate::account_flow::tests::{
        activate, await_register_dialog, create_space, enter_hub, get_json, post_json,
        run_cluster_ceremony, sign_up, successful_body,
    };
    use crate::helpers::{TestEnvironment, driver_with_prf, goto};

    /// The kernels a deployment ships, as its `_host/kernel.json` names them:
    /// a new space's, and a new profile's persona.
    #[derive(Debug, Clone, Deserialize)]
    struct Shipped {
        kernel: String,
        persona: String,
    }

    fn shipped(root: &Path) -> Result<Shipped> {
        let manifest = std::fs::read(root.join("_host/kernel.json"))?;
        Ok(serde_json::from_slice(&manifest)?)
    }

    fn leb128(mut value: u32) -> Vec<u8> {
        let mut encoded = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            encoded.push(byte);
            if value == 0 {
                return encoded;
            }
        }
    }

    /// `kernel`, with a custom section of its own: a component that runs as
    /// it does, by another id.
    fn another_build_of(kernel: &[u8]) -> Vec<u8> {
        let name = b"tonk-integration-kernel-b";
        let mut payload = leb128(name.len() as u32);
        payload.extend_from_slice(name);
        let mut bytes = kernel.to_vec();
        bytes.push(0);
        bytes.extend(leb128(payload.len() as u32));
        bytes.extend(payload);
        bytes
    }

    /// Ship `id`'s kernel again in `root` as another build of it, under its
    /// own id, which it answers.
    fn ship_another(root: &Path, id: &str) -> Result<String> {
        let kernels = root.join("_host/kernel");
        let bytes = std::fs::read(kernels.join(format!("{id}.wasm")))?;
        let another = another_build_of(&bytes);
        let other = tonk_kernel_world::boot::kernel_id(&another);
        ensure!(other != id, "another build of {id} has its id");
        std::fs::write(kernels.join(format!("{other}.wasm")), another)?;
        Ok(other)
    }

    /// Copy what `source` holds into `destination`.
    fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let to = destination.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &to)?;
            } else {
                std::fs::copy(entry.path(), &to)?;
            }
        }
        Ok(())
    }

    /// Make generation B of the deployment: a copy of A that ships another
    /// build of A's kernel for new spaces, and, with `persona`, of its
    /// persona for new profiles. A keeps serving what it served. Answers
    /// what each ships.
    fn prepare_kernel_generation(
        env: &TestEnvironment,
        persona: bool,
    ) -> Result<(Shipped, Shipped)> {
        let generation_a = env.deployment_root.join("generation-a");
        let generation_b = env.deployment_root.join("generation-b");
        ensure!(
            !generation_b.exists(),
            "generation B already exists at {}",
            generation_b.display()
        );
        let a = shipped(&generation_a)?;
        copy_tree(&generation_a, &generation_b)?;
        let mut b = a.clone();
        b.kernel = ship_another(&generation_b, &a.kernel)?;
        if persona {
            b.persona = ship_another(&generation_b, &a.persona)?;
            let shell = generation_b.join("profile.html");
            let page = std::fs::read_to_string(&shell)?;
            let shipping = format!("var SHIPPED_PERSONA = \"{}\";", a.persona);
            ensure!(
                page.contains(&shipping),
                "the profile's shell ships no persona"
            );
            std::fs::write(
                &shell,
                page.replace(
                    &shipping,
                    &format!("var SHIPPED_PERSONA = \"{}\";", b.persona),
                ),
            )?;
        }
        std::fs::write(
            generation_b.join("_host/kernel.json"),
            serde_json::json!({ "kernel": b.kernel, "persona": b.persona }).to_string(),
        )?;
        Ok((a, b))
    }

    /// Have the deployment serve generation B from now on.
    #[cfg(unix)]
    fn promote(env: &TestEnvironment) -> Result<()> {
        use std::os::unix::fs::symlink;

        let next = env.deployment_root.join("current-next");
        let current = env.deployment_root.join("current");
        symlink("generation-b", &next)
            .with_context(|| format!("create deployment link {}", next.display()))?;
        std::fs::rename(&next, &current)
            .with_context(|| format!("promote {} over {}", next.display(), current.display()))?;
        Ok(())
    }

    /// The kernel the worker of each frame of the page runs, by the frame's
    /// origin, as the address the worker was registered at names it, and
    /// whether the frame shows what its kernel answered.
    async fn kernels(driver: &WebDriver) -> Result<BTreeMap<String, (String, bool)>> {
        driver.enter_default_frame().await?;
        let mut found = BTreeMap::new();
        collect(driver, &mut found, 0).await?;
        driver.enter_default_frame().await?;
        Ok(found)
    }

    async fn collect(
        driver: &WebDriver,
        found: &mut BTreeMap<String, (String, bool)>,
        depth: usize,
    ) -> Result<()> {
        if depth > 2 {
            return Ok(());
        }
        let frames = driver.find_all(By::Tag("iframe")).await?;
        for at in 0..frames.len() {
            let frames = driver.find_all(By::Tag("iframe")).await?;
            let Some(frame) = frames.get(at) else { break };
            if frame.clone().enter_frame().await.is_err() {
                continue;
            }
            let seen = driver
                .execute(
                    r#"const worker = navigator.serviceWorker?.controller;
                       const kernel = worker ? new URL(worker.scriptURL).searchParams.get("kernel") : null;
                       const displays = [...document.querySelectorAll("tonk-display")];
                       return {
                           origin: location.origin,
                           kernel,
                           shown: displays.length > 0 && displays.every(display => display.getAttribute("data-state") === "ready"),
                       };"#,
                    Vec::new(),
                )
                .await
                .map(|value| value.json().clone())
                .unwrap_or(Value::Null);
            if let (Some(origin), Some(kernel)) = (seen["origin"].as_str(), seen["kernel"].as_str())
            {
                let shown = seen["shown"].as_bool().unwrap_or(false);
                found.insert(origin.to_owned(), (kernel.to_owned(), shown));
            }
            Box::pin(collect(driver, found, depth + 1)).await?;
            driver.enter_parent_frame().await?;
        }
        Ok(())
    }

    /// Whether `origin` is a space's, not the profile's.
    fn is_space(origin: &str) -> bool {
        !origin
            .split("://")
            .nth(1)
            .unwrap_or(origin)
            .starts_with("profile")
    }

    /// Open the space `key` and wait for it to show what its kernel
    /// answered, under the kernel `kernel`.
    async fn open_on(
        driver: &WebDriver,
        env: &TestEnvironment,
        key: &str,
        kernel: &str,
    ) -> Result<()> {
        goto(driver, env.tonk_web.join(&format!("space/{key}"))?.as_str()).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let running = kernels(driver).await?;
            let up = running
                .iter()
                .any(|(origin, (on, shown))| is_space(origin) && on == kernel && *shown);
            if up {
                return Ok(());
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "the space {key} never came up on the kernel {kernel}: {running:?}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The persona the profile runs.
    async fn persona(driver: &WebDriver) -> Result<String> {
        let running = kernels(driver).await?;
        running
            .iter()
            .find(|(origin, _)| !is_space(origin))
            .map(|(_, (kernel, _))| kernel.clone())
            .with_context(|| format!("the page frames no profile: {running:?}"))
    }

    /// A space keeps the kernel it was made with when the deployment ships
    /// another, and a space made after runs the new one: the page goes from
    /// one to the other and back, each on its own kernel.
    #[dialog_common::test]
    async fn it_runs_each_space_on_the_kernel_it_pins(env: TestEnvironment) -> Result<()> {
        let (a, b) = prepare_kernel_generation(&env, false)?;
        let driver = env.driver().await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let before = create_space(&driver, "Before").await?;
        open_on(&driver, &env, &before, &a.kernel)
            .await
            .context("the space made before the deployment moved")?;

        promote(&env)?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let after = create_space(&driver, "After").await?;
        open_on(&driver, &env, &after, &b.kernel)
            .await
            .context("the space made after the deployment moved")?;
        open_on(&driver, &env, &before, &a.kernel)
            .await
            .context("back to the space made before")?;
        open_on(&driver, &env, &after, &b.kernel)
            .await
            .context("and to the one made after")?;

        driver.quit().await?;
        Ok(())
    }

    /// Accounts switch on a profile that keeps its persona when the
    /// deployment ships another, each opening its own spaces, which run the
    /// kernels they were made with.
    #[dialog_common::test]
    async fn it_switches_accounts_whose_spaces_run_different_kernels(
        env: TestEnvironment,
    ) -> Result<()> {
        let (a, b) = prepare_kernel_generation(&env, true)?;
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "first-kernel@example.com").await?;
        let first = create_space(&driver, "First Kernel").await?;
        open_on(&driver, &env, &first, &a.kernel).await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let first_branch = successful_body("list profiles", &profiles)["active"]
            .as_str()
            .context("no profile is active")?
            .to_owned();

        promote(&env)?;
        let added = post_json(&driver, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add an account", &added);
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        driver
            .find(By::Css("[data-account-trigger]"))
            .await?
            .click()
            .await?;
        await_register_dialog(&driver).await?;
        run_cluster_ceremony(&driver, "second-kernel@example.com").await?;
        activate(&driver, &env, "second-kernel@example.com").await?;
        let second = create_space(&driver, "Second Kernel").await?;
        open_on(&driver, &env, &second, &b.kernel).await?;
        assert_eq!(
            persona(&driver).await?,
            a.persona,
            "the profile keeps the persona it was made with"
        );

        let switched = post_json(
            &driver,
            "/api/profiles/activate",
            serde_json::json!({ "profile": first_branch }),
        )
        .await?;
        successful_body("switch back to the first account", &switched);
        open_on(&driver, &env, &first, &a.kernel)
            .await
            .context("the first account's space")?;
        let listed = get_json(&driver, "/api/profile").await?;
        let keys: Vec<&str> = successful_body("the first account's spaces", &listed)["space"]
            .as_array()
            .map(|spaces| {
                spaces
                    .iter()
                    .filter_map(|space| space["key"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            keys.contains(&first.as_str()) && !keys.contains(&second.as_str()),
            "the first account lists its own space only: {keys:?}"
        );

        driver.quit().await?;
        Ok(())
    }
}
