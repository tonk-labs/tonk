First, read in the BOOTLOAD_CONTEXT.md to understand how this project works.

Now, you will see that I'm using a trick in 404.html to try and parse out the slug and set the
appSlug field in the service-worker through the service-worker's messaging interface. This means
that in order to use the application someone must first go to {hostname}/{appSlug} to register the
slug and then they can begin using the application. This is actually rather brittle. How would you
be able to change the appSlug if you wanted? What if you go to {hostname} right away?

Instead, what we can do is to mimic something similar to how the BIOS is displayed and allows you to
boot up multiple partitions on a computer. When someone goes to {hostname} the page can register the
service worker. Then, once the service worker is registered it can query which apps are available in
the filesystem under the /app path (this is where all apps will be stored). It's a simple
listDirectory call in the VFS under /app.

1. The result of this can then be displayed in the "BIOS" list.
2. The user can use arrow keys (or select with their finger) the application they would like to
   load.
3. A confirmation screen should appear asking if they would really like to open that application
4. If yes, that path is set as appSlug in the service worker
5. Then, the site should redirect to {hostname}/{appSlug}/

The styling and aesthetics of this page should mirror that of a typical BIOS
