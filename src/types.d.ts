declare module "html-to-image" {
  export function toPng(
    node: HTMLElement,
    options?: Record<string, unknown>,
  ): Promise<string>;
}

declare module "*.svg?raw" {
  const content: string;
  export default content;
}
