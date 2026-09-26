// Lets Node load lib/*.ts directly (with --experimental-strip-types), whose
// relative imports leave out the ".ts" that Next's bundler adds itself.
export async function resolve(specifier, context, next) {
  try {
    return await next(specifier, context);
  } catch (err) {
    if (err?.code !== "ERR_MODULE_NOT_FOUND" || !/^\.\.?\//.test(specifier)) throw err;
    return next(`${specifier}.ts`, context);
  }
}
