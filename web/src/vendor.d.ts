/* Ambient declarations for the vendored craft-agents-oss UI tree.
 *
 * tsconfig.app.json excludes `src/vendor` from compilation, but as soon as a
 * file inside `src/` imports `~vendor/...` tsc would otherwise resolve and
 * type-check the actual `.tsx` files (defeating the exclusion). Declaring
 * the alias prefix as ambient `any`-typed modules tells tsc "trust the
 * runtime", while vite still bundles the real source.
 */
declare module '~vendor/*';
declare module '@craft-agent/*';
declare module '@config/*';

declare module '*.svg' {
  const src: string;
  export default src;
}
