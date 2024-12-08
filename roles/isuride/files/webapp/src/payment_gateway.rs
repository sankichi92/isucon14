use crate::models::Ride;
use crate::Error;
use std::future::Future;

#[derive(Debug, thiserror::Error)]
pub enum PaymentGatewayError {
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("unexpected number of payments: {ride_count} != {payment_count}.")]
    UnexpectedNumberOfPayments {
        ride_count: usize,
        payment_count: usize,
    },
    #[error("[GET /payments] unexpected status code ({0})")]
    GetPayment(reqwest::StatusCode),
}

#[derive(Debug, serde::Serialize)]
pub struct PaymentGatewayPostPaymentRequest {
    pub amount: i32,
}

#[derive(Debug, serde::Deserialize)]
struct PaymentGatewayGetPaymentsResponseOne {
    amount: i32,
    status: String,
}

pub trait PostPaymentCallback<'a> {
    type Output: Future<Output = Result<Vec<Ride>, Error>>;

    fn call(&self, tx: &'a mut sqlx::MySqlConnection, user_id: &'a str) -> Self::Output;
}
impl<'a, F, Fut> PostPaymentCallback<'a> for F
where
    F: Fn(&'a mut sqlx::MySqlConnection, &'a str) -> Fut,
    Fut: Future<Output = Result<Vec<Ride>, Error>>,
{
    type Output = Fut;
    fn call(&self, tx: &'a mut sqlx::MySqlConnection, user_id: &'a str) -> Fut {
        self(tx, user_id)
    }
}

pub async fn request_payment_gateway_post_payment(
    payment_gateway_url: &str,
    token: &str,
    param: &PaymentGatewayPostPaymentRequest,
) -> Result<(), Error>
{
    // 失敗したらとりあえずリトライ
    // FIXME: 社内決済マイクロサービスのインフラに異常が発生していて、同時にたくさんリクエストすると変なことになる可能性あり
    let mut retry = 0;

    let idempotency_key = ulid::Ulid::new().to_string();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Idempotency-Key",
        reqwest::header::HeaderValue::from_str(&idempotency_key).unwrap(),
    );

    loop {
        let result = async {
            let res = reqwest::Client::new()
                .post(format!("{payment_gateway_url}/payments"))
                .bearer_auth(token)
                .headers(headers.clone())
                .json(param)
                .send()
                .await
                .map_err(PaymentGatewayError::Reqwest)?;

            if res.status() != reqwest::StatusCode::NO_CONTENT {
                return Err(PaymentGatewayError::GetPayment(res.status()).into());
            }
            Ok(())
        }
        .await;

        if let Err(err) = result {
            if retry < 5 {
                retry += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            } else {
                return Err(err);
            }
        }
        break;
    }

    Ok(())
}
