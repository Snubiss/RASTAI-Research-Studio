// Test support: puts the app's stylesheets on the test page, the way main.jsx loads them: @rastai/ui's two, then the
// frame's. The dev server serves them from their folders, as packages/ui's own tests load theirs.

const STYLESHEETS = [
  new URL("../../../../packages/ui/src/styles/base.css", import.meta.url).href,
  new URL("../../../../packages/ui/src/styles/components.css", import.meta.url).href,
  new URL("../app/frame.css", import.meta.url).href,
];

/**
 * Adds each stylesheet once and waits until all of them have loaded.
 */
export async function loadAppStyles() {
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
