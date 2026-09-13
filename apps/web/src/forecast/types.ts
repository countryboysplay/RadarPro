// Plain TypeScript mirrors of `crates/forecast-web/src/wasm_api.rs`'s JSON
// wire shapes -- this file has no logic of its own, matching this
// codebase's "no logic of its own in the browser glue" convention (see
// `apps/web/src/alerts/types.ts`): `forecast-core`/`provider-gefs`/
// `provider-hrrr` decide everything about discovery, decoding, and units;
// this app only ever renders what `ProviderHandle` hands back.

/** The two concrete providers `forecast-web`'s `ProviderHandle` accepts. */
export type ForecastProviderId = "gefs" | "hrrr";

/** Mirrors `ModelMetadataJson` -- a provider's static identity/description,
 * available with no network call via `ProviderHandle.metadata()`. */
export interface ForecastModelMetadata {
  providerId: string;
  displayName: string;
  /** `true` for GEFS (ensemble), `false` for HRRR (deterministic) -- the
   * UI must not offer an ensemble-statistic picker when this is `false`
   * (Global Contract). */
  isEnsemble: boolean;
  resolutionDescription: string;
}

/** Mirrors `RunMetadataJson` -- the run `discoverLatestRun` resolved to. */
export interface ForecastRunMetadata {
  providerId: string;
  /** ISO-8601 UTC timestamp. */
  initTime: string;
  label: string;
}

/** Mirrors `NativeVariableMetadataJson`. */
export interface ForecastNativeVariableMetadata {
  providerVariableName: string;
  providerLevelName: string;
  nativeUnit: string;
}

/** Mirrors `EnsembleJson` (`#[serde(tag = "kind")]`) -- `null` on the outer
 * `ForecastGridMetadata.ensemble` means a deterministic provider (HRRR). */
export type ForecastEnsembleJson =
  | { kind: "control" }
  | { kind: "member"; value: number }
  | { kind: "mean" }
  | { kind: "percentile"; value: number };

/** Mirrors `GeometryJson` (`#[serde(tag = "kind")]`). */
export type ForecastGeometryJson =
  | {
      kind: "regularLatLon";
      width: number;
      height: number;
      originLatDeg: number;
      originLonDeg: number;
      latStepDeg: number;
      lonStepDeg: number;
    }
  | {
      kind: "lambertConformal";
      width: number;
      height: number;
      originXM: number;
      originYM: number;
      dxM: number;
      dyM: number;
    };

/** Mirrors `ForecastGridMetadataJson` -- resolved by `fetchField`.
 * Deliberately excludes the decoded value array itself (see that method's
 * own doc comment in `wasm_api.rs`); render it via `renderCurrentGrid`. */
export interface ForecastGridMetadata {
  variable: string;
  native: ForecastNativeVariableMetadata;
  providerId: string;
  unit: string;
  /** ISO-8601 UTC timestamp. */
  runTime: string;
  forecastLeadHours: number;
  /** ISO-8601 UTC timestamp. */
  validTime: string;
  ensemble: ForecastEnsembleJson | null;
  geometry: ForecastGeometryJson;
}

/** The `ensemble` argument `ProviderHandle.fetchField` accepts -- one of
 * `"control"`, `"mean"`, `"member:<n>"`, `"percentile:<n>"`, or
 * `null`/`undefined` for a deterministic provider (HRRR). Kept as a plain
 * string union (not a richer discriminated type) since this is exactly the
 * wire format `parse_ensemble` (`wasm_api.rs`) expects -- no translation
 * layer needed. */
export type ForecastEnsembleSpec = "control" | "mean" | `member:${number}` | `percentile:${number}`;
