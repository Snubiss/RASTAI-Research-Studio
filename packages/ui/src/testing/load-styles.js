// Test support: puts @rastai/ui's two stylesheets on the test page with <link> elements, the way an app's page
// loads them, and waits until both have loaded. A stylesheet already on the page is not added twice.

const STYLESHEETS = [
  new URL("../styles/base.css", import.meta.url).href,
  new URL("../styles/components.css", import.meta.url).href,
];

export async function loadStyles() {
  for (const href of STYLESHEETS) {
    if (document.querySelector(`link[href="${href}"]`)) {
      continue;
    }
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = href;
    const loaded = new Promise((resolve, reject) => {
      link.addEventListener("load", resolve);
      link.addEventListener("error", () => reject(new Error(`could not load ${href}`)));
    });
    document.head.append(link);
    await loaded;
  }
}
