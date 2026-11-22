declare module '*.po' {
  const catalog: Record<string, string | string[]>;
  export default catalog;
}
