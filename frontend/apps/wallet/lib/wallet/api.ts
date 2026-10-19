import type {
  PublicKeyCredentialCreationOptionsJSON,
  RegistrationResponseJSON,
} from "@simplewebauthn/browser";
import type { Address } from "viem";
import { RP_ID, RP_NAME } from "./config";

/** 后端登记过的钱包 Passkey：凭证 ID 与公钥（SPKI DER），都是 base64url。 */
export interface StoredPasskey {
  credentialId: string;
  publicKey: string;
  rpId: string;
}

/**
 * 钱包前端依赖的后端接口（forum-api 正在实现：challenge 存 Valkey 5 分钟，webauthn-rs 验证）。
 * 页面只依赖这个接口；接入真实 API 时新增一个实现替换 `walletApi`，不改调用方。
 */
export interface WalletApi {
  /** 当前登录用户已开通的钱包 Passkey；未开通返回 null */
  getPasskey(): Promise<StoredPasskey | null>;
  /** 开通交易第 1 步：后端生成一次性 challenge 与创建参数 */
  registrationOptions(): Promise<PublicKeyCredentialCreationOptionsJSON>;
  /** 开通交易第 2 步：后端验证注册结果并保存凭证 */
  verifyRegistration(response: RegistrationResponseJSON): Promise<StoredPasskey>;
  /** 开通交易第 3 步：前端算出的智能账户地址写入 wallets 表 */
  saveWalletAddress(address: Address): Promise<void>;
}

const STORE_KEY = "akari.mock-wallet-api.v1";

interface MockState {
  userId: string;
  userName: string;
  challenge?: string;
  passkey?: StoredPasskey;
  address?: Address;
}

function randomBase64Url(bytes: number): string {
  const buf = crypto.getRandomValues(new Uint8Array(bytes));
  return btoa(String.fromCharCode(...buf))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");
}

function decodeBase64Url(s: string): string {
  return atob(s.replaceAll("-", "+").replaceAll("_", "/"));
}

/**
 * 后端就绪前的模拟实现：状态存在本浏览器 localStorage（每个浏览器 / Playwright 上下文各是一个「用户」）。
 * 只做 challenge 与类型核对，不验签——真正的验签在后端 webauthn-rs。
 */
export function createMockWalletApi(storage: Storage = localStorage): WalletApi {
  const load = (): MockState => {
    const raw = storage.getItem(STORE_KEY);
    if (raw) return JSON.parse(raw) as MockState;
    return { userId: randomBase64Url(16), userName: `user-${randomBase64Url(3)}` };
  };
  const save = (s: MockState) => storage.setItem(STORE_KEY, JSON.stringify(s));

  return {
    async getPasskey() {
      return load().passkey ?? null;
    },
    async registrationOptions() {
      const s = { ...load(), challenge: randomBase64Url(32) };
      save(s);
      return {
        challenge: s.challenge,
        rp: { id: RP_ID, name: RP_NAME },
        // 密码管理器里一眼可辨（plan 第 5 节）
        user: {
          id: s.userId,
          name: s.userName,
          displayName: `Akari 钱包签名（勿删）· ${s.userName}`,
        },
        // 只接受 P-256（ES256）：链上 WebAuthn 验证器只认这条曲线
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        authenticatorSelection: { residentKey: "required", userVerification: "required" },
        attestation: "none",
        timeout: 60_000,
      };
    },
    async verifyRegistration(response) {
      const s = load();
      const clientData = JSON.parse(decodeBase64Url(response.response.clientDataJSON)) as {
        type: string;
        challenge: string;
      };
      if (
        !s.challenge ||
        clientData.type !== "webauthn.create" ||
        clientData.challenge !== s.challenge
      ) {
        throw new Error("Passkey 注册校验失败，请重试");
      }
      if (!response.response.publicKey)
        throw new Error("这个浏览器没有返回 Passkey 公钥，请换用最新版浏览器");
      const passkey = {
        credentialId: response.id,
        publicKey: response.response.publicKey,
        rpId: RP_ID,
      };
      save({ userId: s.userId, userName: s.userName, passkey });
      return passkey;
    },
    async saveWalletAddress(address) {
      save({ ...load(), address });
    },
  };
}

let instance: WalletApi | undefined;

/** 当前使用的实现。后端接口上线后在这里切换。 */
export function walletApi(): WalletApi {
  instance ??= createMockWalletApi();
  return instance;
}
