use reqwest::{
    Response, StatusCode,
    multipart::{self, Part},
};

#[derive(thiserror::Error, Debug)]
pub enum PastebinError {
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("failed response: {0:?}")]
    FailedResponse(Response),
}

pub async fn url(
    client: &reqwest::Client,
    file_name: &str,
    data: Vec<u8>,
) -> Result<String, PastebinError> {
    let form = multipart::Form::new().part("c", Part::bytes(data).file_name(file_name.to_string()));
    let response = client
        .post("https://pb.li7g.com")
        .multipart(form)
        .send()
        .await?;
    if response.status() == StatusCode::OK {
        Ok(response.text().await?.trim().to_string())
    } else {
        Err(PastebinError::FailedResponse(response))
    }
}

pub async fn curl_command(
    client: &reqwest::Client,
    file_name: &str,
    data: Vec<u8>,
) -> Result<String, PastebinError> {
    url(client, file_name, data)
        .await
        .map(|s| format!("curl {s}"))
}
